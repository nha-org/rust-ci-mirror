//! Refuses `cargo test` in a workspace that runs on nextest (its root holds `.config/nextest.toml`: rust/), before it
//! runs: TestBudget needs each test in its own process (rust/CLAUDE.md rule 1), and three turns
//! were lost to it in a week (task rust-claude-hooks-port).
use super::shell::{Call, LEAD, code, segments};
use regex::Regex;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

/// cargo as a word anywhere in a simple command (after bin/bounded, env, nice...), its options (a global one's value too),
/// then test or its alias t
static CARGO_TEST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?:^|[\s/(])cargo(?:\s+(?:(?:--manifest-path|--config|--color|--target-dir|-Z|-C)(?:=\S+|\s+\S+)|[-+]\S+))*\s+t(?:est)?(?:[\s)]|$)",
    )
    .expect("cargo test")
});
static CD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"{LEAD}cd(\s.*)?$")).expect("cd"));
/// where cargo reads its workspace from: a manifest's folder, or -C's
static MANIFEST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"--manifest-path(?:=|\s+)(\S+)").expect("manifest"));
static CHDIR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s-C(?:=|\s+)(\S+)").expect("-C"));

pub const MESSAGE: &str = "Refused by dash hook cargo-test: this workspace runs its tests with `cargo nextest run` (rust/CLAUDE.md rule 1): TestBudget needs each test in its own process, and cargo test fails them by turns. Run `cargo nextest run -p <crate> [<filter>]` instead.";

pub fn refused(call: &Call) -> Option<String> {
    // the folder each simple command runs in: the call's, moved by each `cd`; None while a path is unreadable
    let mut dir = Some(call.cwd.to_path_buf());
    // the folder before each open subshell, back when it closes
    let mut saved = vec![];
    for seg in segments(&code(call.command)) {
        let opens = seg
            .chars()
            .filter(|c| !c.is_whitespace())
            .take_while(|&c| c == '(')
            .count();
        for _ in 0..opens {
            saved.push(dir.clone());
        }
        if let Some(m) = CD.captures(&seg) {
            dir = cd(
                dir.as_deref(),
                m.get(1).map_or("", |g| g.as_str()),
                call.home,
            );
        } else if CARGO_TEST.is_match(&seg) {
            let at = match (MANIFEST.captures(&seg), CHDIR.captures(&seg)) {
                (Some(m), _) => resolve(dir.as_deref(), &m[1], call.home)
                    .and_then(|f| f.parent().map(Path::to_path_buf)),
                (None, Some(m)) => resolve(dir.as_deref(), &m[1], call.home),
                (None, None) => dir.clone(),
            };
            // ponytail: a folder it cannot read (a quote, a variable) is taken as a nextest one; bash -c '...' is not read
            let nextest = at.is_none_or(|at| {
                at.ancestors()
                    .any(|a| (call.is_file)(&a.join(".config/nextest.toml")))
            });
            if nextest {
                return Some(MESSAGE.into());
            }
        }
        let closes = seg
            .chars()
            .rev()
            .filter(|c| !c.is_whitespace())
            .take_while(|&c| c == ')')
            .count();
        for _ in 0..closes {
            if let Some(before) = saved.pop() {
                dir = before;
            }
        }
    }
    None
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
fn resolve(dir: Option<&Path>, path: &str, home: &Path) -> Option<PathBuf> {
    if path.contains(['$', '`', '\'', '"']) {
        return None;
    }
    let joined = match path.strip_prefix('~') {
        Some("") => home.to_path_buf(),
        Some(rest) if rest.starts_with('/') => home.join(&rest[1..]),
        _ if path.starts_with('/') => PathBuf::from(path),
        _ => dir?.join(path),
    };
    let mut out = PathBuf::new();
    for part in joined.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// The guard on `cmd` run from `cwd`, the nextest workspace at /repo/rust.
    fn judge(cmd: &str, cwd: &str) -> Option<String> {
        let list = || Ok(String::new());
        let call = Call {
            tool: "Bash",
            command: cmd,
            file: "",
            cwd: Path::new(cwd),
            project: Path::new("/repo"),
            home: Path::new("/home"),
            panes: &list,
            is_file: &|p| p == Path::new("/repo/rust/.config/nextest.toml"),
            is_dir: &|_| true,
            read: &|_| None,
            real: &|p| p.to_path_buf(),
        };
        refused(&call)
    }

    #[dash_test]
    fn refuses_cargo_test_under_rust_naming_nextest() {
        for (cmd, cwd) in [
            ("cargo test -p dash-gate", "/repo/rust"),
            ("cargo test", "/repo/rust/crates/gate"),
            ("cd rust && cargo test -p dash", "/repo"),
            ("cd /repo/rust; cargo +nightly test", "/tmp"),
            ("(cd rust/crates/dash && cargo t)", "/repo"),
            (
                "~/repo/site_perso/bin/bounded 60 cargo --locked test",
                "/repo/rust",
            ),
            ("cargo test --manifest-path rust/Cargo.toml", "/repo"),
            ("CARGO_TARGET_DIR=x cargo test", "/repo/rust"),
            (
                "cd \"$CLAUDE_PROJECT_DIR/rust\" && cargo test",
                "/elsewhere",
            ),
            // /code-review of PR 1457: options with a value, a redirected cd, a subshell's cd left behind
            ("cargo --manifest-path rust/Cargo.toml test", "/repo"),
            ("cargo -Z unstable test", "/repo/rust"),
            ("cargo -C rust test", "/repo"),
            ("cd rust 2>/dev/null && cargo test", "/repo"),
            ("(cd /other && make); cargo test", "/repo/rust"),
        ] {
            let why = judge(cmd, cwd).unwrap_or_else(|| panic!("{cmd} from {cwd}"));
            assert!(why.contains("cargo nextest run -p <crate>"), "{why}");
        }
    }

    #[dash_test]
    fn passes_nextest_and_cargo_test_outside_a_nextest_workspace() {
        for (cmd, cwd) in [
            ("cargo nextest run -p dash-gate", "/repo/rust"),
            ("cargo test", "/other/project"),
            ("cd /other && cargo test", "/repo/rust"),
            ("cargo build --tests", "/repo/rust"),
            ("git commit -m 'never cargo test'", "/repo/rust"),
            ("grep -rn 'cargo test' .", "/repo/rust"),
            ("cargo test --manifest-path /other/Cargo.toml", "/repo/rust"),
            ("cargo testsuite", "/repo/rust"),
            // an absolute cd after an unreadable one; a bare cd goes home
            ("cd \"$TMPDIR/x\" && cd /other && cargo test", "/repo/rust"),
            ("cd && cargo test", "/repo/rust"),
        ] {
            assert_eq!(judge(cmd, cwd), None, "{cmd} from {cwd}");
        }
    }
}
