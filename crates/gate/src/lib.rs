//! Tooling, outside the rings of section 3: the rules CI holds the workspace to. crates/gate/tests runs them.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub mod graph;
pub mod html;

/// Section 3: at most this much Rust (@nha, 2026-11-04: « 50k seems like a max »).
pub const LINE_LIMIT: usize = 50_000;

/// The crates of section 3's table, its binary, and this tooling.
pub const RINGS: [&str; 11] = [
    "core", "store", "views", "index", "ingest", "ml", "jobs", "sync", "surface", "ui", "ext",
];
pub const BINARY: &str = "dash";
/// `process` is the bounded call the layers on a command line tool share: it reads a clock and starts threads, which the core
/// may not (a seed reruns the core), and it sits on no port. `statsd` is the tool's own numbers, the binary's alone. `site`
/// is nicolas-ha.com's server and image, a program of its own on Fly that takes nothing of ours (task rust-site-no-container).
pub const TOOLING: [&str; 5] = ["meter", "gate", "process", "statsd", "site"];
/// Section 3's last row, what phases 1 and 2 build of it, each on a core port. Its own list: rust_budgets_test.exs holds RINGS
/// to the rows of one crate each. `bridge` leaves with the older app's part (section 20b).
pub const TASK_LAYERS: [&str; 7] = [
    "orch", "terminal", "launcher", "agent", "policy", "forge", "bridge",
];
/// Section 20b's plugins: a feature on the core's ports alone, compiled in. `drafts` keeps each draft and its outcome,
/// `comments` queues each comment of his for the orchestrator's wake.
pub const PLUGINS: [&str; 2] = ["drafts", "comments"];
/// Section 20b's rule: a program or provider is a plugin, so the core's source never names one.
pub const NEVER_IN_THE_CORE: [&str; 11] = [
    "herdr",
    "tmux",
    "claude",
    "anthropic",
    "codex",
    "gemini",
    "make",
    "gh",
    "datadog",
    "statsd",
    "dd",
];

/// The only edges between rings besides the one to the core, as section 3 names them.
const NAMED_EDGES: [(&str, &str); 8] = [
    ("surface", "views"),
    ("ui", "views"),
    ("jobs", "ml"),
    ("index", "store"),
    ("terminal", "process"),
    ("forge", "process"),
    ("bridge", "process"),
    ("agent", "process"),
];

/// What is wrong in these normal and build dependencies, `from` and `to` without the dash- prefix: the core takes nothing
/// of ours, the rest take the core and section 3's named edges. The binary wires them all, so it is exempt.
pub fn bad_edges(edges: &[(String, String)]) -> Vec<String> {
    edges
        .iter()
        .filter(|(from, to)| {
            let (from, to) = (from.as_str(), to.as_str());
            from != BINARY
                && !(from != "core" && to == "core")
                && !NAMED_EDGES.contains(&(from, to))
        })
        .map(|(from, to)| format!("{from} -> {to}"))
        .collect()
}

/// Every test takes the meter as a dev-dependency, so that one is no edge in either reader of the graph.
pub fn is_tooling_edge(dev: bool, to: &str) -> bool {
    dev && to == "meter"
}

/// A dependency loop in these edges, as its path. Cargo refuses one only among normal dependencies: a dev-dependency can
/// close one.
pub fn dependency_loop(edges: &[(String, String)]) -> Option<Vec<String>> {
    /// `done` holds what has no loop behind it: without it, the walk doubles with each pair of paths that meet again.
    fn walk(
        at: &str,
        edges: &[(String, String)],
        path: &mut Vec<String>,
        done: &mut BTreeSet<String>,
    ) -> Option<Vec<String>> {
        if let Some(start) = path.iter().position(|p| p == at) {
            return Some([&path[start..], &[at.to_string()]].concat());
        }
        if done.contains(at) {
            return None;
        }
        path.push(at.to_string());
        let mut next = edges.iter().filter(|(from, _)| from == at);
        let found = next.find_map(|(_, to)| walk(to, edges, path, done));
        path.pop();
        done.insert(at.to_string());
        found
    }
    let mut done = BTreeSet::new();
    let mut starts = edges.iter();
    starts.find_map(|(from, _)| walk(from, edges, &mut vec![], &mut done))
}

/// The names of `NEVER_IN_THE_CORE` this text holds as whole words, in any case: « Makefile » is not « make ».
pub fn tools_named(text: &str) -> Vec<&'static str> {
    let named = |tool: &&str| {
        let mut words = text.split(|c: char| !c.is_alphanumeric());
        words.any(|w| w.eq_ignore_ascii_case(tool))
    };
    NEVER_IN_THE_CORE.into_iter().filter(named).collect()
}

/// Section 20b phase 3: the agent kinds orch's code never names, so a new agent changes no orch line (@nha, 2026-11-05:
/// « replaceable just means clean interface. layers. deps graph that makes sense. no loops. enforce this by a test. »).
/// `anthropic` is Claude by its provider (/code-review).
pub const AGENT_KINDS: [&str; 4] = ["claude", "anthropic", "codex", "gemini"];

/// Every .rs under `root`, build output left out.
pub fn rust_files(root: &Path) -> Vec<PathBuf> {
    let mut found = vec![];
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).expect("read dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n != "target") {
                    dirs.push(path);
                }
            } else if path.extension().is_some_and(|e| e == "rs") {
                found.push(path);
            }
        }
    }
    found
}

pub fn line_count(files: &[PathBuf]) -> usize {
    files
        .iter()
        .map(|f| std::fs::read_to_string(f).expect("read").lines().count())
        .sum()
}

/// Every file git tracks under `root`, `(mode, path)` with the path from `root`: what CrmGoneTest's `git grep` and
/// `mix lint`'s script count read of rust/ (task rust-gate-mirrors-repo-scans).
pub fn tracked(root: &Path) -> Vec<(String, String)> {
    let out = std::process::Command::new("git")
        .args(["ls-files", "--stage", "-z"])
        .current_dir(root)
        .output()
        .expect("git ls-files");
    assert!(out.status.success(), "git ls-files: {:?}", out.status);
    // A name that is not UTF-8 fails here, never escapes the scans (/code-review).
    let listing = String::from_utf8(out.stdout).expect("git ls-files: a path that is not UTF-8");
    // Regular files only, as the lint reads them: a symlink is no script, and its target is not its text (/code-review).
    let mut files: Vec<(String, String)> = listing
        .split('\0')
        .filter_map(|entry| entry.split_once('\t'))
        .map(|(stage, path)| (stage[..6].to_string(), path.to_string()))
        .filter(|(mode, path)| {
            ["100644", "100755"].contains(&mode.as_str()) && root.join(path).is_file()
        })
        .collect();
    // a conflicted path once, as the lint reads it
    files.dedup_by(|a, b| a.1 == b.1);
    files
}

/// The old CRM's folder and its app's name, split so this source names neither: CrmGoneTest greps the whole repository.
pub const OLD_CRM: [&str; 2] = [concat!("private", "/crm"), concat!("crm", "-ui")];

/// CrmGoneTest's rule on rust/ (CRM-GONE.md step 1, his words quoted in crm_gone_test.exs): the files naming `OLD_CRM`'s
/// folder, or its app as its own word (a task id ending in it names a task), in any case. A .jsonl is a
/// journal, quoting old names as history: left out there too. #1180's fixture named the folder and main went red.
pub fn old_crm_named(root: &Path, files: &[(String, String)]) -> Vec<String> {
    let names = |text: &[u8]| {
        let text = text.to_ascii_lowercase();
        let [folder, app] = OLD_CRM.map(str::as_bytes);
        let own_word =
            |i: usize| i == 0 || !(text[i - 1] == b'-' || text[i - 1].is_ascii_lowercase());
        text.windows(folder.len()).any(|w| w == folder)
            || text
                .windows(app.len())
                .enumerate()
                .any(|(i, w)| w == app && own_word(i))
    };
    files
        .iter()
        .filter(|(_, path)| !path.ends_with(".jsonl"))
        .filter(|(_, path)| names(&std::fs::read(root.join(path)).expect("read")))
        .map(|(_, path)| path.clone())
        .collect()
}

/// The one script rust/ holds, a checkout's way in that builds the tool before it runs (task rust-design-page; @nha, 2026-11-08:
/// « lmk how to run it. ./rust/bin/dash serve or similar »): mix lint's held count took it in the commit that added it.
pub const HELD_SCRIPT: &str = "bin/dash";

/// `mix lint`'s script rule on rust/ (Dashboard.Lint.scripts, task no-new-python): a tracked script raises the repository's
/// held count, and a file opening on #! without its executable bit is refused, a `#![…]` attribute on a .rs's first line
/// included. Rust holds none but HELD_SCRIPT, kept executable, and its CI's, an executable `ci/*.sh`, each counted in the
/// lint's held number (@nha, 2026-11-08: « make the CI stuff logic mostly in bash scripts please, avoid inline yaml
/// scripts »): new code is Rust or the Elixir app.
pub fn scripts(root: &Path, files: &[(String, String)]) -> Vec<String> {
    let mut bad = vec![];
    // held only as a script: without its executable bit it is refused as any #! file is
    let held = |mode: &str, path: &str| {
        mode == "100755"
            && (path == HELD_SCRIPT || (path.starts_with("ci/") && path.ends_with(".sh")))
    };
    for (mode, path) in files.iter().filter(|(mode, path)| !held(mode, path)) {
        let ext = Path::new(path).extension().and_then(|e| e.to_str());
        let mut head = [0u8; 2];
        let shebang = std::fs::File::open(root.join(path))
            .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head))
            .is_ok()
            && head == *b"#!";
        if mode == "100755" || matches!(ext, Some("py" | "sh" | "bash" | "zsh")) {
            bad.push(format!("{path}: a script"));
        } else if shebang {
            bad.push(format!("{path}: opens on #! without its executable bit"));
        }
    }
    bad
}

/// The steps of a workflow whose `run:` is a block of lines: CI logic is a script (the mirror's ci/, site_perso's
/// bin/ci/), one line a step (@nha, 2026-11-08: « make the CI stuff logic mostly in bash scripts please, avoid inline
/// yaml scripts »). A block is `|` or `>`, or a plain scalar carried on a deeper line (check.yml held three).
pub fn inline_runs(workflow: &str) -> Vec<String> {
    let lines: Vec<&str> = workflow.lines().collect();
    let indent = |l: &str| l.len() - l.trim_start().len();
    (0..lines.len())
        .filter(|&i| {
            // a list item's dash, then any spaces: `-   run: |` is a step too (/code-review)
            let l = lines[i].trim_start();
            let l = l.strip_prefix('-').map_or(l, str::trim_start);
            let Some(v) = l.strip_prefix("run:") else {
                return false;
            };
            let key = lines[i].len() - l.len();
            let next = lines[i + 1..]
                .iter()
                .find(|n| !n.trim().is_empty() && !n.trim_start().starts_with('#'));
            // an empty value is a mapping, as `defaults: run: shell:`
            let v = v.trim();
            v.starts_with(['|', '>']) || !v.is_empty() && next.is_some_and(|n| indent(n) > key)
        })
        .map(|i| format!("line {}: a run of several lines, make it a script", i + 1))
        .collect()
}

/// Dashboard.Lint's `@deno` exclusions; workspace_rules.rs's the_deno_copies_are_mix_lints holds the two equal.
pub const DENO_RULES_EXCLUDE: &str = "--rules-exclude=no-window,no-window-prefix,require-await";

/// `mix lint`'s JS half (Dashboard.Lint's `@deno`, task hooks-tested-linted) on rust/'s tracked .js, .mjs, .ts and .tsx: its
/// findings, each a line of deno's, or why deno did not run. The mirror's CI installs the same pinned deno as bin/ci/lint-tools.sh.
pub fn deno_lint(root: &Path, files: &[(String, String)]) -> Vec<String> {
    let js: Vec<&str> = files
        .iter()
        .map(|(_, path)| path.as_str())
        .filter(|path| {
            let ext = Path::new(path).extension().and_then(|e| e.to_str());
            matches!(ext, Some("js" | "mjs" | "ts" | "tsx"))
        })
        .collect();
    if js.is_empty() {
        return vec![];
    }
    let mut deno = std::process::Command::new("deno");
    deno.args(["lint", "--quiet", DENO_RULES_EXCLUDE])
        .args(&js)
        .env("NO_COLOR", "1")
        .current_dir(root);
    findings(deno, "mix lint's JS half needs it on PATH")
}

/// Task rust-fmt-gate: the tracked .rs files rustfmt would change, each a line, or why rustfmt did not run. Every
/// workspace under rust/ is edition 2024, so one call with this rustfmt holds them all. rust/rustfmt.toml keeps a
/// personal ~/.config/rustfmt out of it; rustfmt's output is stable within a style edition, so the Mac's and the
/// runner's stable agree without a pinned toolchain (/code-review).
pub fn rustfmt_check(root: &Path, files: &[(String, String)]) -> Vec<String> {
    let rs: Vec<&str> = files
        .iter()
        .map(|(_, path)| path.as_str())
        .filter(|path| path.ends_with(".rs"))
        .collect();
    if rs.is_empty() {
        return vec![];
    }
    let mut rustfmt = std::process::Command::new("rustfmt");
    rustfmt
        .args(["--check", "--edition", "2024", "--files-with-diff"])
        .args(&rs)
        .current_dir(root);
    let mut found = findings(rustfmt, "rustup component add rustfmt");
    // a module is named as itself and again through its parent's `mod`: each file once, an error's lines untouched
    let mut seen = std::collections::HashSet::new();
    found.retain(|line| !line.ends_with(".rs") || seen.insert(line.clone()));
    found
}

/// A checker's run: nothing on success, else its output's lines, or why it did not run, with `hint` the way to get it.
fn findings(mut command: std::process::Command, hint: &str) -> Vec<String> {
    let program = command.get_program().to_string_lossy().into_owned();
    match command.output() {
        Err(e) => vec![format!("{program}: {e} ({hint})")],
        Ok(out) if out.status.success() => vec![],
        Ok(out) => {
            let text = [out.stdout, out.stderr].concat();
            let lines: Vec<String> = String::from_utf8_lossy(&text)
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(str::to_string)
                .collect();
            if lines.is_empty() {
                vec![format!("{program} failed: {:?}", out.status)]
            } else {
                lines
            }
        }
    }
}
