//! Tooling, outside the rings of section 3: the rules CI holds the workspace to. crates/gate/tests runs them.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub mod affected;
pub mod binary;
pub mod browser;
pub mod build_paths;
pub mod firings;
pub mod graph;
pub mod guard;
pub mod hook;
pub mod html;
pub mod inline_js;
pub mod lint;
pub mod merge;
pub mod ratchet;
pub mod served;
pub mod tool_flags;

/// Section 3's size target, in tokens (`graph::token_count`, so a line break or a comment never moves it): 50,000 lines at
/// main's 7.22 tokens a line on 2026-11-09 (@nha, 2026-11-04: « 50k seems like a max »). Shown on /deps, never a gate
/// (@nha, 2026-11-09: « we need visuals to understand too. lift the ban to lines. we're doing this together now. »).
pub const TOKEN_TARGET: usize = 360_768;

/// The crates of section 3's table, its binary, and this tooling.
pub const RINGS: [&str; 11] = [
    "core", "store", "views", "index", "ingest", "ml", "jobs", "sync", "surface", "ui", "ext",
];
pub const BINARY: &str = "dash";
/// `process` is the bounded call the layers on a command line tool share: it reads a clock and starts threads, which the core
/// may not (a seed reruns the core), and it sits on no port. `statsd` is the tool's own numbers, the binary's alone. `site`
/// is nicolas-ha.com's server and image, a program of its own on Fly that takes nothing of ours (task rust-site-no-container).
/// `held-debs` installs the mirror's Ubuntu packages before any build that links them (task rust-ci-apt-retry). `test-macro`
/// is `#[dash_test]`, which the meter re-exports: a proc macro needs a crate of its own (task rust-test-budget-attr).
pub const TOOLING: [&str; 7] = [
    "meter",
    "gate",
    "process",
    "statsd",
    "site",
    "held-debs",
    "test-macro",
];
/// Section 18's leaf crates: a ring's heavy dependency in a crate of its own, so a change elsewhere neither rebuilds nor
/// tests it (`dash ci affected`). `pdf` is ingest's PDF reader, `ocr` its OCR, empty until rust-pdf-ocr-tables (task
/// rust-heavy-crates-split; @nha, 2026-11-11: « cutting internal crates for internal OCR/search etc. »).
pub const LEAVES: [&str; 2] = ["pdf", "ocr"];
/// Section 3's last row, what phases 1 and 2 build of it, each on a core port. Its own list: rust_budgets_test.exs holds RINGS
/// to the rows of one crate each. `bridge` leaves with the older app's part (section 20b).
pub const TASK_LAYERS: [&str; 7] = [
    "orch", "terminal", "launcher", "agent", "policy", "forge", "bridge",
];
/// Section 20b's plugins: a feature on the core's ports alone, compiled in. `drafts` keeps each draft and its outcome,
/// `comments` queues each comment of his for the orchestrator's wake, `mail` reads klar's mail server and draws /mail,
/// `hooks` answers Claude Code's hooks, the binary handing it its clock, repo and Jev client (task rust-hooks-rest-port).
pub const PLUGINS: [&str; 4] = ["drafts", "comments", "mail", "hooks"];
/// The plugin crates rfc-rust-plugins-ui's port map moves the binary's modules into before they exist (task rust-port-map):
/// each leaves this list for [`PLUGINS`] in its move, which the port map's test holds.
pub const PLANNED_PLUGINS: [&str; 7] =
    ["board", "hunt", "daily", "docs", "search", "linear", "ops"];
/// The job hunt's words, which mail's source never holds (rfc-rust-plugins-ui section 3; @nha, 2026-11-08: « mail should not
/// know about jobs imo and mail can be a plugin »).
pub const JOB_WORDS: [&str; 8] = ["job", "jobs", "lead", "leads", "ad", "ads", "cv", "cvs"];
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
const NAMED_EDGES: [(&str, &str); 11] = [
    ("surface", "views"),
    ("ui", "views"),
    ("jobs", "ml"),
    ("index", "store"),
    ("terminal", "process"),
    ("forge", "process"),
    ("bridge", "process"),
    ("agent", "process"),
    ("hooks", "process"),
    // a leaf of section 18 plugs into its ring's registry
    ("pdf", "ingest"),
    // the meter's `dash_test` attribute, every test's
    ("meter", "test-macro"),
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

/// Every test takes the meter as a dev-dependency, so that one is no edge in either reader of the graph. A plugin's test may
/// also draw its UI tree with the design system, its HTML and its text form, as the page does: no code of the plugin takes it
/// (task rust-mail-design-parts, mail's inbox.rs). Any other crate's stays an edge.
pub fn is_tooling_edge(from: &str, dev: bool, to: &str) -> bool {
    dev && (to == "meter" || (to == "surface" && PLUGINS.contains(&from)))
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
    words_named(text, &NEVER_IN_THE_CORE)
}

/// Each of `words` this text holds as a whole word, in any case.
pub fn words_named(text: &str, words: &[&'static str]) -> Vec<&'static str> {
    let named = |word: &&str| {
        let mut said = text.split(|c: char| !c.is_alphanumeric());
        said.any(|w| w.eq_ignore_ascii_case(word))
    };
    words.iter().copied().filter(named).collect()
}

/// Section 20b phase 3: the agent kinds orch's code never names, so a new agent changes no orch line (@nha, 2026-11-05:
/// « replaceable just means clean interface. layers. deps graph that makes sense. no loops. enforce this by a test. »).
/// `anthropic` is Claude by its provider (/code-review).
pub const AGENT_KINDS: [&str; 4] = ["claude", "anthropic", "codex", "gemini"];

/// Task rust-older-markers-one-reader: the older store's markers, U+E000 and U+E001, are read in `dash_store::older` alone
/// (rust-judge-on-arrival lost two parity runs to a second reader in chances.rs).
pub const MARKERS_HOME: &str = "crates/store/src/older.rs";

/// The lines of `source`, from 1, holding U+E000 or U+E001 as Rust, JSON or SQL spells it, in any case, `_` ignored: the
/// character; inside a token, `\u{..}` (leading zeros too), `\u....` and the escaped UTF-8 bytes; as a whole token (a Rust
/// integer suffix allowed), the decimal number (`char(..)`, `char::from_u32(..)`, `unicode(..) = ..`) and the hex number
/// (leading zeros too); the bytes' hex at a byte's place in a blob (`x'..'`). A wider hex word (`0xE000_0000`) and a colour
/// (six hex digits after `#`) are no match. A blank ends a token (/code-review: blanks dropped, the number before « AND »
/// or « as u32 » passed). ponytail: a byte array (`[0xee, 0x80, 0x80]`) passes, and the decimal number is refused
/// whatever it counts (a size of 56 KiB is written `56 * 1024`); OPEN and CLOSE being private is the mechanism, this scan
/// the net behind it.
pub fn marker_literals(source: &str) -> Vec<usize> {
    // each code point from its neighbour, so no token of this file is one of them
    let points = [0xDFFF_u32 + 1, 0xDFFF + 2];
    let chars: Vec<char> = points.iter().filter_map(|&p| char::from_u32(p)).collect();
    let (mut inside, mut tokens, mut blobs) = (vec![], vec![], vec![]);
    for &c in &chars {
        let p = c as u32;
        let mut utf8 = [0; 4];
        let bytes = c.encode_utf8(&mut utf8).as_bytes();
        let escaped: String = bytes.iter().map(|b| format!("\\x{b:02x}")).collect();
        inside.extend([format!("\\u{{{p:x}}}"), format!("\\u{p:x}"), escaped]);
        blobs.push(bytes.iter().map(|b| format!("{b:02x}")).collect::<String>());
        tokens.extend([p.to_string(), format!("0x{p:x}")]);
    }
    let suffixes = [
        "", "u16", "u32", "u64", "u128", "usize", "i32", "i64", "i128", "isize",
    ];
    let normal = |line: &str| {
        let mut low: String = (line.chars())
            .filter(|c| *c != '_')
            .flat_map(char::to_lowercase)
            .collect();
        for (zero, none) in [("\\u{0", "\\u{"), ("0x0", "0x")] {
            while low.contains(zero) {
                low = low.replace(zero, none);
            }
        }
        low
    };
    // a blob's hex digits, `x'..'`, holding a marker's bytes at an even place
    let blob = |low: &str| {
        low.split("x'").skip(1).any(|after| {
            let hex = (after.split(|c: char| !c.is_ascii_hexdigit()).next()).unwrap_or_default();
            (0..hex.len())
                .step_by(2)
                .any(|at| blobs.iter().any(|b| hex[at..].starts_with(b.as_str())))
        })
    };
    let token = |t: &str| {
        (tokens.iter()).any(|k| {
            t.strip_prefix(k.as_str())
                .is_some_and(|s| suffixes.contains(&s))
        })
    };
    (source.lines().enumerate())
        .filter(|(_, line)| {
            let low = normal(line);
            line.contains(&chars[..])
                || inside.iter().any(|s| low.contains(s.as_str()))
                || low.split(|c: char| !c.is_ascii_alphanumeric()).any(token)
                || blob(&low)
        })
        .map(|(i, _)| i + 1)
        .collect()
}

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
/// The index's regular files, `(mode, path)`, read as `lint`'s git reads them (a checkout under a git dir reads its own index), on disk or not: a hosted sparse checkout lacks private/docs
/// and the replicator, which `lint::browser_launch_own_gone` must still see tracked (/code-review of PR 1519). An error, never a
/// panic; [`tracked`] is this, on disk.
pub fn index(root: &Path) -> Result<Vec<(String, String)>, String> {
    let mut git = std::process::Command::new("git");
    git.current_dir(root);
    if root.components().any(|c| c.as_os_str() == ".git") {
        git.env_remove("GIT_INDEX_FILE");
    }
    let out = git
        .args(["ls-files", "--stage", "-z"])
        .output()
        .map_err(|e| format!("git ls-files: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git ls-files: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let listing = String::from_utf8(out.stdout)
        .map_err(|_| "git ls-files: a path that is not UTF-8".to_string())?;
    let mut files: Vec<(String, String)> = listing
        .split('\0')
        .filter_map(|entry| entry.split_once('\t'))
        .map(|(stage, path)| (stage[..6].to_string(), path.to_string()))
        .filter(|(mode, _)| ["100644", "100755"].contains(&mode.as_str()))
        .collect();
    // a conflicted path once, as the lint reads it
    files.dedup_by(|a, b| a.1 == b.1);
    Ok(files)
}

/// Of `index`, the files on disk: the ones a rule can read.
pub fn on_disk(root: &Path, index: &[(String, String)]) -> Vec<(String, String)> {
    index
        .iter()
        .filter(|(_, p)| root.join(p).is_file())
        .cloned()
        .collect()
}

/// Regular files only, as the lint reads them ([`index`], the one reader): a symlink is no script, and its target is not
/// its text; a name that is not UTF-8 fails here, never escapes the scans (/code-review).
pub fn tracked(root: &Path) -> Vec<(String, String)> {
    let mut files = on_disk(root, &index(root).expect("git ls-files"));
    // a vendored crate stays as it was published plus its patch: never ours to format or rule (task rust-ui-hit-panic)
    files.retain(|(_, path)| !path.starts_with("vendor/"));
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

pub use tool_flags::{BROWSER_LAUNCH, DENO_RULES_EXCLUDE};

/// `mix lint`'s browser slot rule on rust/ (`Dashboard.Lint.browser_slots/3`, task browser-slots-everywhere): a file that
/// starts a browser takes one of the machine's slots through bin/browser-slot. Markdown names a launch and runs none; a
/// file that is not text launches nothing. Here so a commit touching only rust/ keeps it without the app's lint (task
/// brief-tests-prepush-skip: that lint took minutes in a fresh worktree).
pub fn browsers_without_slot(root: &Path, files: &[(String, String)]) -> Vec<String> {
    let launches = |text: &str| {
        BROWSER_LAUNCH.iter().any(|p| {
            text.match_indices(p).any(|(i, _)| {
                let next = text[i + p.len()..].chars().next();
                *p != BROWSER_LAUNCH[4] || next.is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
            })
        })
    };
    let slot = concat!("browser", "-slot");
    files
        .iter()
        .filter(|(_, path)| !path.ends_with(".md"))
        .filter(|(_, path)| {
            let text = std::fs::read_to_string(root.join(path)).unwrap_or_default();
            launches(&text) && !text.contains(slot)
        })
        .map(|(_, path)| path.clone())
        .collect()
}

/// The one script rust/ holds, a checkout's way in that builds the tool before it runs (task rust-design-page; @nha, 2026-11-08:
/// « lmk how to run it. ./rust/bin/dash serve or similar »): mix lint's held count took it in the commit that added it.
pub const HELD_SCRIPT: &str = "bin/dash";

/// `mix lint`'s script rule on rust/ (Dashboard.Lint.scripts, task no-new-python): a tracked script raises the repository's
/// held count, and a file opening on #! without its executable bit is refused; `#![…]`, Rust's inner attribute, is no #!
/// (a vendored crate's `#![allow(..)]`, task rust-ui-hit-panic), as in the lint's git grep. Rust holds none but
/// HELD_SCRIPT, kept executable, and its CI's, an executable `ci/*.sh`, each counted in the
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
        let mut head = Vec::with_capacity(3);
        let _ = std::fs::File::open(root.join(path))
            .and_then(|f| std::io::Read::read_to_end(&mut std::io::Read::take(f, 3), &mut head));
        // one definition of a script for rust/ and the rest (ratchet's)
        if ratchet::named(mode, path) {
            bad.push(format!("{path}: a script"));
        } else if ratchet::shebang(&head) {
            bad.push(format!("{path}: opens on #! without its executable bit"));
        }
    }
    bad
}

/// The files under crates/ that git takes for binary, a NUL byte in their first 8000: a PDF, an image, a model. A text file
/// in another charset, as a reader's Latin-1 page, is text (/code-review). A crate's heavy
/// test file is listed in its fixtures.txt and fetched through `dash_meter::fixtures`' sha256-checked cache, never tracked
/// (task rust-heavy-crates-split; @nha, 2026-11-11: « There is a real risk that the test time balloons with OCR or other
/// data-heavy tasks »).
pub fn binary_fixtures(root: &Path, files: &[(String, String)]) -> Vec<String> {
    files
        .iter()
        .filter(|(_, path)| path.starts_with("crates/"))
        .filter(|(_, path)| {
            let mut head = Vec::with_capacity(8000);
            let file = std::fs::File::open(root.join(path)).expect("open");
            std::io::Read::read_to_end(&mut std::io::Read::take(file, 8000), &mut head)
                .expect("read");
            head.contains(&0)
        })
        .map(|(_, path)| format!("{path}: binary"))
        .collect()
}

/// Each line of a workflow or of a script its steps run that calls apt with no held .debs first (task rust-ci-apt-retry):
/// a package goes through the `held-debs` binary, whose `dpkg -i` of the held set comes before apt (its crate's test). An
/// Ubuntu outage (2026-11-11, 13:50 to 17:05) then turns no run red. A comment, whole line or trailing, is never a call.
pub fn apt_in_workflow(workflow: &str) -> Vec<String> {
    workflow
        .lines()
        .enumerate()
        .filter(|(_, l)| {
            let code = l.split(" #").next().unwrap_or_default();
            !code.trim_start().starts_with('#')
                && code
                    .split(|c: char| c.is_whitespace() || ";&|()\"'".contains(c))
                    .any(|w| matches!(w, "apt" | "apt-get" | "aptitude"))
        })
        .map(|(i, _)| format!("line {}: apt with no held .debs first", i + 1))
        .collect()
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
pub(crate) fn findings(mut command: std::process::Command, hint: &str) -> Vec<String> {
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

/// The files that may reach past rust/ at run time: the one helper, which says None on the mirror.
pub const REACHES_PAST_RUST: [&str; 1] = ["crates/meter/src/lib.rs"];

/// Each code line of a tracked .rs file outside REACHES_PAST_RUST that reaches past rust/ at run time (`CARGO_MANIFEST_DIR`,
/// then a `"../../..` or `/../../..` path on its code line or the next three, as rustfmt wraps a `concat!` and as a `"{}`
/// placeholder leads a `format!`): the mirror holds
/// rust/ alone, so there the path is missing and the test fails a CI round later (ship.rs read private/dashboard/bin/dash,
/// PR 1426; a wrapped `concat!` read fill-merge.js, PR 1480, 2026-11-12). Fix: `dash_meter::monorepo()`. ponytail: a reach
/// built from `rust_root().join("..")` is not seen; the mirror catches it.
pub fn run_time_paths_past_rust(root: &Path, files: &[(String, String)]) -> Vec<String> {
    let dir = concat!("CARGO_", "MANIFEST_DIR");
    let past = [concat!("\"../", "../.."), concat!("/../", "../..")];
    let mut bad = vec![];
    for (_, file) in files {
        if !file.ends_with(".rs") || REACHES_PAST_RUST.contains(&file.as_str()) {
            continue;
        }
        let text =
            std::fs::read_to_string(root.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
        let lines: Vec<&str> = text.lines().collect();
        for (n, line) in lines.iter().enumerate() {
            let window = &lines[n..lines.len().min(n + 4)];
            if !line.trim_start().starts_with("//")
                && line.contains(dir)
                && window.iter().any(|l| {
                    !l.trim_start().starts_with("//") && past.iter().any(|p| l.contains(p))
                })
            {
                bad.push(format!(
                    "{file}:{}: reaches past rust/; take dash_meter::monorepo()",
                    n + 1
                ));
            }
        }
    }
    bad
}

/// Each file a mirrored .rs compiles in (`include_str!`, `include_bytes!`, `include!`) that the mirror does not carry:
/// tracked, under a line of `mirror` (ci-mirror.txt, read as Dashboard.CiMirror reads it: a path or a folder, with or
/// without its `/`). It builds on this Mac and fails only on the mirror, a CI round later (rust/parity.tsv, PR 1407,
/// 2026-11-11). ponytail: a path built by `concat!`/`env!` and a file read at run time are not seen; the mirror catches them.
pub fn includes_off_the_mirror(
    root: &Path,
    files: &[(String, String)],
    mirror: &str,
) -> Vec<String> {
    let lines: Vec<&str> = mirror
        .lines()
        .map(|l| l.trim().trim_end_matches('/'))
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    let listed = |path: &str| {
        lines.iter().any(|l| {
            path == *l
                || path
                    .strip_prefix(l)
                    .is_some_and(|rest| rest.starts_with('/'))
        })
    };
    let tracked: BTreeSet<&str> = files.iter().map(|(_, f)| f.as_str()).collect();
    let mut bad = vec![];
    for file in tracked.iter().filter(|f| f.ends_with(".rs") && listed(f)) {
        let text = std::fs::read_to_string(root.join(file)).unwrap_or_default();
        // a comment line names an include without compiling one in; rustfmt may carry the path to the next line
        let code: String = text
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        // split, so this file's own text names no include
        let opens = [
            concat!("include_str", "!("),
            concat!("include_bytes", "!("),
            concat!("include", "!("),
        ];
        for macro_open in opens {
            for rest in code.split(macro_open).skip(1) {
                let rest = rest
                    .trim_start()
                    .trim_start_matches('r')
                    .trim_start_matches('#');
                let Some((target, _)) = rest.strip_prefix('"').and_then(|r| r.split_once('"'))
                else {
                    continue;
                };
                // lexically, from the file's folder
                let mut parts: Vec<&str> = file.split('/').collect();
                parts.pop();
                let mut inside = true;
                for part in target.split('/') {
                    match part {
                        "." | "" => {}
                        ".." => inside &= parts.pop().is_some(),
                        p => parts.push(p),
                    }
                }
                let path = parts.join("/");
                let why = if !inside {
                    "is outside rust/"
                } else if !tracked.contains(path.as_str()) {
                    "is not tracked"
                } else if !listed(&path) {
                    "is not on ci-mirror.txt's list"
                } else {
                    continue;
                };
                bad.push(format!("{file}: {target} {why}"));
            }
        }
    }
    bad
}
