//! Refuses a `cargo nextest run` stress (`--stress-count`, `--stress-duration`) that names no test: neither an `-E` filter
//! nor a test name. rust-front-door-filing-race ran the whole workspace 20 times: load 95, the PM loop's passes killed twice
//! (task stress-run-scoped; @nha, 2026-11-12: « how can we have less defects like this one ? »). A `-p <crate>` alone still
//! runs every test of the crate each round, so it is refused too.
use super::shell::{Call, code, spans};
use regex::Regex;
use std::sync::LazyLock;

/// cargo (or the cargo-nextest binary), its own options, then nextest run (or its alias r); the run's words follow the match
static RUN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?:^|[\s/(])cargo(?:-nextest)?(?:\s+(?:(?:--manifest-path|--config|--color|--target-dir|-Z|-C)(?:=\S+|\s+\S+)|[-+]\S+))*\s+nextest\s+r(?:un)?(?:\s|$)",
    )
    .expect("nextest run")
});

/// an -E filter's value as typed, quoted or not
static FILTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:^|\s)(?:-E|--filterset|--filter-expr)(?:=|\s*)('[^']*'?|"[^"]*"?|\S+)"#)
        .expect("filter")
});

/// what makes a filter select more than its named tests: a negation, a difference, every test
static WIDENS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bnot\b|!|\)\s*-|all\(\)").expect("widens"));

/// a filter's union operators, nextest's `|`, `+` and `or`
static UNION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\||\+|\bor\b").expect("union"));

/// nextest run's options that take a value as the next word (`--opt=v` is one word either way), as `cargo nextest run
/// --help` lists them (0.9.151), then libtest's after `--`
const VALUED: &[&str] = &[
    "-p",
    "--package",
    "-P",
    "--profile",
    "-j",
    "--test-threads",
    "--jobs",
    "-F",
    "--features",
    "-R",
    "--rerun",
    "-Z",
    "--archive-file",
    "--archive-format",
    "--bench",
    "--bin",
    "--binaries-metadata",
    "--build-dir-remap",
    "--build-jobs",
    "--cargo-message-format",
    "--cargo-metadata",
    "--cargo-profile",
    "--color",
    "--config",
    "--config-file",
    "--debugger",
    "--example",
    "--exclude",
    "--extract-to",
    "--failure-output",
    "--final-status-level",
    "--flaky-result",
    "--manifest-path",
    "--max-fail",
    "--max-progress-running",
    "--message-format",
    "--message-format-version",
    "--no-tests",
    "--partition",
    "--platform-filter",
    "--retries",
    "--run-ignored",
    "--show-progress",
    "--status-level",
    "--stress-count",
    "--stress-duration",
    "--success-output",
    "--target",
    "--target-dir",
    "--target-dir-remap",
    "--test",
    "--tool-config-file",
    "--tracer",
    "--user-config-file",
    "--workspace-remap",
    "--skip",
    "--format",
    "--logfile",
    "--shuffle-seed",
];

pub const MESSAGE: &str = "Refused by dash hook nextest-stress: a stress run names its test. Without an -E filter or a test name every selected test runs each round, all cores at once (rust-front-door-filing-race ran the workspace 20 times: load 95, the PM loop's passes killed twice). Run `cargo nextest run -p <crate> <test name> --stress-count 20`, or `-E 'test(<name>)'` for several.";

pub fn refused(call: &Call) -> Option<String> {
    // code() blanks a quoted filter, so its value is read from the same span of the command as typed
    let code = code(call.command);
    let (blanked, typed): (Vec<char>, Vec<char>) =
        (code.chars().collect(), call.command.chars().collect());
    spans(&code)
        .into_iter()
        .any(|r| {
            let typed = typed
                .get(r.clone())
                .map_or(String::new(), |t| t.iter().collect());
            unscoped_stress(&blanked[r].iter().collect::<String>(), &typed)
        })
        .then(|| MESSAGE.into())
}

/// A filter naming tests only: each alternative holds `test(`, and nothing widens it (`not test(slow)`, `all() - test(x)`).
fn scopes(filter: &str) -> bool {
    !WIDENS.is_match(filter) && UNION.split(filter).all(|alt| alt.contains("test("))
}

/// A nextest run in `seg` (`typed`: the same span, quotes kept) with a stress option and neither -E filters all naming
/// tests (nextest runs their union) nor a test name.
fn unscoped_stress(seg: &str, typed: &str) -> bool {
    let Some(m) = RUN.find(seg) else {
        return false;
    };
    let after: String = typed.chars().skip(seg[..m.end()].chars().count()).collect();
    let filters: Vec<String> = FILTER
        .captures_iter(&after)
        .map(|c| c[1].to_string())
        .collect();
    let tests = !filters.is_empty() && filters.iter().all(|f| scopes(f));
    let (mut stress, mut named) = (false, false);
    let mut words = seg[m.end()..].split_whitespace();
    while let Some(w) = words.next() {
        let flag = w.split('=').next().unwrap_or(w);
        if flag.starts_with("--stress") {
            stress = true;
        }
        if ["-E", "--filterset", "--filter-expr"].contains(&flag)
            || w.starts_with("-E") && w.len() > 2
        {
            named |= tests;
            if ["-E", "--filterset", "--filter-expr"].contains(&w) {
                words.next(); // its value, read from the command as typed above
            }
        } else if w.contains(['<', '>']) {
            // a redirection, its target the next word when it ends there
            if w.ends_with(['<', '>']) {
                words.next();
            }
        } else if VALUED.contains(&w) {
            words.next();
        } else if w.starts_with(|c: char| c.is_alphanumeric() || "_:'\"".contains(c)) {
            // a test name, before `--` or after it (libtest's filter); a `)` closing a subshell is none
            named = true;
        }
    }
    stress && !named
}

/// dash task add's problems with its done-when checks, one per check that asks for a stress (`--stress-count`, « 20 runs ») across the
/// full parallel run, not one test (task stress-run-scoped; @nha, 2026-11-12: « how can we have less defects like this one
/// ? »). A check that says what refuses one (a guard's own done-when) asks for none. A word holding stress (nextest-stress)
/// is none either (/code-review medium), nor a count under 5, a CI rerun (/code-review high). Here, beside the guard,
/// since the dash binary only shrinks (rust-binary-ratchet).
/// ponytail: English words only; a check written another way passes, and dash hook nextest-stress still refuses the run.
pub fn done_when_problems(checks: &[String]) -> Vec<String> {
    static STRESS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?i)--stress|(?:^|[^\w-])stress(?:[^\w-]|$)|\b(?:[5-9]|\d{2,})\s*(?:runs|passes)\b",
        )
        .expect("a stress")
    });
    static REFUSES: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)\b(?:refus|den(?:y|ies)|block)").expect("a refusal"));
    static FULL: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\bfull\s+(?:parallel\s+)?(?:run|suite)|\bwhole\s+(?:workspace|suite|run)|--workspace|workspace[- ]wide|\b(?:every|all(?:\s+the)?)\s+tests?\b")
            .expect("a full run")
    });
    let full = |c: &str| STRESS.is_match(c) && FULL.is_match(c) && !REFUSES.is_match(c);
    (1..).zip(checks).filter(|(_, c)| full(c)).map(|(n, _)| {
        format!(
            ":done-when check {n}: a stress in the full parallel run loads every core (rust-front-door-filing-race: load 95, \
             the PM loop killed twice); name its test, « cargo nextest run -p <crate> <test name> --stress-count 20 », the line dash \
             hook nextest-stress lets run"
        )
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;
    use std::path::Path;

    fn judge(cmd: &str) -> Option<String> {
        let list = || Ok(String::new());
        refused(&Call {
            tool: "Bash",
            command: cmd,
            file: "",
            cwd: Path::new("/repo/rust"),
            project: Path::new("/repo"),
            home: Path::new("/home"),
            panes: &list,
            is_file: &|_| false,
            is_dir: &|_| true,
            read: &|_| None,
            real: &|p| p.to_path_buf(),
        })
    }

    /// Done-when 1: a stress with no -E filter and no test name is refused, the message naming the line to run.
    #[dash_test]
    fn a_stress_naming_no_test_is_refused_with_the_line_to_run() {
        for cmd in [
            "cargo nextest run --stress-count 20",
            "cargo nextest run --workspace --stress-count 20",
            "cargo nextest run -p dash --stress-count=20",
            "cd rust && cargo nextest run -p dash-gate --stress-duration 5m 2>&1 | tail -20",
            "~/repo/site_perso/bin/bounded 600 cargo nextest run --locked -j 4 --stress-count 20 > log",
            "cargo --locked nextest r --stress-count 20 -- --exact",
            "bash -c 'cargo nextest run --stress-count 20'",
            // /code-review medium: a filter selecting everything or a package, a valued option, libtest's --skip, a
            // closing paren, a cargo option with a separate value
            "cargo nextest run --workspace -E 'all()' --stress-count 20",
            "cargo nextest run -E 'package(dash)' --stress-count 20",
            "cargo nextest run --workspace --stress-count 20 --no-tests warn",
            "cargo nextest run --stress-count 20 -- --skip slow",
            "(cd rust && cargo nextest run -p dash --stress-count 20 )",
            "cargo --color always nextest run --workspace --stress-count 20",
            // /code-review high: a filter widened by not, a difference or a union, -E text of another command or of
            // a message, a valued option missing before, libtest's --format, the cargo-nextest binary
            "cargo nextest run --workspace -E 'not test(slow)' --stress-count 20",
            "cargo nextest run -E 'all() - test(x)' --stress-count 20",
            "cargo nextest run -E 'package(dash) | test(x)' --stress-count 20",
            "cargo nextest run -E 'test(x)' -E 'package(dash)' --stress-count 20",
            "grep -E 'test(' src.rs; cargo nextest run -E 'all()' --stress-count 20",
            "git commit -m \"use -E 'test(x)'\" && cargo nextest run -E 'all()' --stress-count 20",
            "cargo nextest run --workspace --show-progress none --stress-count 20",
            "cargo nextest run --stress-count 20 -- --format json",
            "cargo-nextest nextest run --workspace --stress-count 20",
            "~/.cargo/bin/cargo-nextest nextest run --stress-count 20",
        ] {
            let why = judge(cmd).unwrap_or_else(|| panic!("passed: {cmd}"));
            assert!(
                why.starts_with("Refused by dash hook nextest-stress:")
                    && why.contains("cargo nextest run -p <crate> <test name> --stress-count 20"),
                "{why}"
            );
        }
    }

    /// A stress naming its test or a filter passes, and so does a run with no stress, or text naming one.
    #[dash_test]
    fn a_stress_naming_its_test_or_a_filter_passes() {
        for cmd in [
            "cargo nextest run -p dash not_built_yet --stress-count 20",
            "cargo nextest run -p dash --stress-count 20 not_built_yet",
            "cargo nextest run -p dash -E 'test(not_built_yet)' --stress-count 20",
            "cargo nextest run -p dash --filterset=test(x) --stress-count 20",
            "cargo nextest run -E \"package(dash) & test(x)\" --stress-count 20",
            "cargo nextest run -E 'test(a) | test(b)' -E 'test(c)' --stress-count 20",
            "cargo nextest run -p dash-gate -E 'package(dash-gate) & test(not_built_yet)' --stress-count 20",
            "cargo nextest run -p dash 'not_built_yet' --stress-count 20",
            "cargo nextest run -p dash --stress-count 20 -- --exact extensions::not_built_yet",
            "cargo nextest run --workspace",
            "cargo nextest run -p dash -j 4",
            "git commit -m 'never cargo nextest run --stress-count 20'",
        ] {
            assert_eq!(judge(cmd), None, "{cmd}");
        }
    }
}
