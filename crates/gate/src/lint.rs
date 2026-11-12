//! `mix lint`'s checks (Dashboard.Lint and Mix.Tasks.Lint, ported in task rust-lint-port; @nha, 2026-11-10: « no diffs on ex
//! or exs at all, rust HAS to take over »), each a rule over the repository's tracked files, `root` its top: `dash lint` runs
//! them all, the pre-commit's lint step runs the tools on the staged files and the rest on the whole tree. Each rule returns
//! its findings, one line each, empty when it holds; crates/gate/tests/lint_rules.rs holds each to its Elixir original.
//! The script ratchet and shellcheck are ratchet.rs's, run here as `dash scripts count --check` runs them. Only what reads
//! Rust, docs or scripts is ported (the orchestrator's word, 2026-11-12: « Port only what reads Rust, docs or scripts »):
//! no-elixir refuses any changed .ex or .exs, so a rule reading only the app's Elixir could never fire again. The db
//! timeouts, file server, EDN fixture and private read rules stayed behind, and so did live-data-test --check.
use regex::Regex;
use std::path::Path;
use std::process::Command;

/// A regex compiled once (outbound.rs's own): `prose` and the rules run on every document of every commit.
macro_rules! re {
    ($s:expr) => {{
        static R: std::sync::LazyLock<Regex> =
            std::sync::LazyLock::new(|| Regex::new($s).expect($s));
        &*R
    }};
}

/// git in `root`; a checkout under a git dir (the pre-commit's staged one) reads its own index, never the hook's
/// GIT_INDEX_FILE, as `hook::Ctx::cmd` holds it (/code-review).
fn git(root: &Path) -> Command {
    let mut c = Command::new("git");
    c.current_dir(root);
    if root.components().any(|c| c.as_os_str() == ".git") {
        c.env_remove("GIT_INDEX_FILE");
    }
    c
}

/// The tracked regular files, `(mode, path)`, as `crate::tracked` reads them, read with [`git`]; an error, never a panic.
pub fn tracked(root: &Path) -> Result<Vec<(String, String)>, String> {
    let out = git(root)
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
        .filter(|(mode, path)| {
            ["100644", "100755"].contains(&mode.as_str()) && root.join(path).is_file()
        })
        .collect();
    files.dedup_by(|a, b| a.1 == b.1);
    Ok(files)
}

/// A tracked file's text, empty when it is not UTF-8 (a binary names nothing).
fn read(root: &Path, path: &str) -> String {
    std::fs::read_to_string(root.join(path)).unwrap_or_default()
}

fn ext_in(path: &str, exts: &[&str]) -> bool {
    let ext = Path::new(path).extension().and_then(|e| e.to_str());
    ext.is_some_and(|e| exts.contains(&e))
}

/// The tracked paths under one of `dirs` whose extension is one of `exts`.
fn under<'a>(files: &'a [(String, String)], dirs: &[&str], exts: &[&str]) -> Vec<&'a str> {
    files
        .iter()
        .map(|(_, p)| p.as_str())
        .filter(|p| dirs.iter().any(|d| p.starts_with(d)) && ext_in(p, exts))
        .collect()
}

/// « path:line » of each line of each file that `hit` holds.
fn lines_hit(root: &Path, paths: &[&str], hit: impl Fn(&str) -> bool) -> Vec<String> {
    let mut out = vec![];
    for p in paths {
        for (i, l) in read(root, p).lines().enumerate() {
            if hit(l) {
                out.push(format!("{p}:{}", i + 1));
            }
        }
    }
    out
}

/// Downward obliviousness (RFC orchestration, section 1; @nha, 2026-10-17: « gate this. Is it respected right now ? »): the
/// harness never names the orchestration. `(warnings, errors)`: the approved sandbox.sb only warns, since no session can
/// change it; the Makefile and sandbox/jobs.conf (nor the tasks either) and sandbox.sb.candidate fail.
pub fn harness(root: &Path) -> (Vec<String>, Vec<String>) {
    let orch = r"(?i)\bao\b|AO_|orchestrat|crm\.pm|task.run|pm.loop|session\.sh delegate";
    let hits = |files: &[&str], rx: &str| {
        let rx = Regex::new(rx).expect("harness regex");
        lines_hit(root, files, |l| rx.is_match(l))
    };
    let strict = [
        hits(
            &["Makefile", "sandbox/jobs.conf"],
            &format!(r"{orch}|(?i)\btasks?\b"),
        ),
        hits(&["sandbox.sb.candidate"], orch),
    ]
    .concat();
    (hits(&["sandbox.sb"], orch), strict)
}

/// A raw hex color in a Rust page (design.json: « no raw colors »): the design system, crates/surface/, owns every color as a
/// token. A letter a-f after the hash makes a color; three or six digits only after « : » or a quote, since a hash and
/// digits in a comment name an issue or PR. The old CRM's .ex and .heex, which mix lint read, are frozen by no-elixir and read no more (the orchestrator's word,
/// 2026-11-12: « Port only what reads Rust, docs or scripts »).
pub fn hex_colors(root: &Path, files: &[(String, String)]) -> Vec<String> {
    let lettered = re!(r"#([0-9a-fA-F]{3,8})\b");
    let digits = re!(r#"[:"']\s*#(?:[0-9]{3}|[0-9]{6})\b"#);
    let paths: Vec<&str> = under(files, &["rust/crates/"], &["rs"])
        .into_iter()
        .filter(|p| !p.starts_with("rust/crates/surface/"))
        .collect();
    lines_hit(root, &paths, |l| {
        digits.is_match(l)
            || lettered
                .captures_iter(l)
                .any(|c| c[1].chars().any(|ch| ch.is_ascii_alphabetic()))
    })
}

/// His words cited as @nha, never « Nicolas, <date> » (@nha, 2026-10-05: « "@nha" rule »), in the Rust and the scripts
/// (a .py, .sh or .bash, or a file opening on #!), ours and editable; the app's frozen Elixir, which mix lint read, no more.
pub fn citations(root: &Path, files: &[(String, String)]) -> Vec<String> {
    let rx = re!(r"Nicolas, 20[0-9]{2}-[0-9]{2}-[0-9]{2}");
    let paths: Vec<&str> = files
        .iter()
        .map(|(_, p)| p.as_str())
        .filter(|p| crate::ratchet::ours(p))
        .filter(|p| {
            ext_in(p, &["rs", "py", "sh", "bash"])
                || (Path::new(p).extension().is_none() && read(root, p).starts_with("#!"))
        })
        .collect();
    lines_hit(root, &paths, |l| rx.is_match(l))
}

/// An AI tell: its case-insensitive regex, its label and what to write instead, as the outbound gate reads the table.
pub type Tell = (Regex, String, String);

/// Dashboard.Docs.html_unescape: numeric entities, then the five named ones a document holds, `&amp;` last.
fn unescape(s: &str) -> String {
    let num = re!(r"&#(x[0-9a-fA-F]+|[0-9]+);");
    let s = num.replace_all(s, |c: &regex::Captures| {
        let n = &c[1];
        let code = match n.strip_prefix('x') {
            Some(h) => u32::from_str_radix(h, 16).ok(),
            None => n.parse().ok(),
        };
        code.and_then(char::from_u32)
            .map_or_else(|| c[0].to_string(), String::from)
    });
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&nbsp;", "\u{a0}")
        .replace("&amp;", "&")
}

/// A document's prose: `data-verbatim` elements (his text kept as he wrote it), code, scripts, styles and diagrams left
/// out, then the tags, entities decoded. ponytail: an element ends at the first closing tag of its name, so a nested one
/// cuts it short, as Dashboard.AiIsms's lazy match did; a parser if a verbatim element ever nests its own kind.
pub fn prose(html: &str) -> String {
    let mut text = html.to_string();
    let verbatim = re!(r"(?s)<(\w+)\b[^>]*\bdata-verbatim\b[^>]*>");
    // an element with no closing tag is left, as the Elixir match left it, and the search goes on past its `<`
    let mut from = 0;
    while let Some(c) = verbatim.captures_at(&text, from) {
        let open = c.get(0).expect("match").range();
        let close = format!("</{}>", &c[1]);
        let end = text[open.end..]
            .find(&close)
            .map(|i| open.end + i + close.len());
        if let Some(end) = end {
            text.replace_range(open.start..end, " ");
        }
        from = open.start + 1;
    }
    let elements = [
        re!(r"(?is)\x3Cscript\b.*?\x3C/script>"),
        re!(r"(?is)\x3Cstyle\b.*?\x3C/style>"),
        re!(r"(?is)\x3Ccode\b.*?\x3C/code>"),
        re!(r"(?is)\x3Cpre\b.*?\x3C/pre>"),
        re!(r"(?is)\x3Csvg\b.*?\x3C/svg>"),
    ];
    for rx in elements {
        text = rx.replace_all(&text, " ").into_owned();
    }
    let tag = re!(r"<[^>]+>");
    unescape(&tag.replace_all(&text, " "))
}

/// A quote is someone's words, a `code span` a name: « », “ ”, "…" and backticks are left out.
pub fn unquoted(text: &str) -> String {
    let rx = re!(r#"«[^»]*»|“[^”]*”|"[^"\n]{0,80}"|`[^`\n]*`"#);
    rx.replace_all(text, " ").into_owned()
}

/// The tells in plain text, one line each: « the match »: label, write instead.
pub fn tells_in(text: &str, tells: &[Tell]) -> Vec<String> {
    let text = unquoted(text);
    tells
        .iter()
        .filter_map(|(rx, label, instead)| {
            rx.find(&text)
                .map(|m| format!("« {} »: {label}, write {instead}", m.as_str().trim()))
        })
        .collect()
}

/// The AI tells banned in everything we write (@nha, 2026-10-22, on rfc-dashboard: « "stays with him" em-dashes
/// "load-bearing" "it's not X it's Y" and other AI-isms are banned everywhere. »), in each document of private/docs.
pub fn ai_isms_docs(root: &Path, files: &[(String, String)], tells: &[Tell]) -> Vec<String> {
    let mut docs = under(files, &["private/docs/"], &["html"]);
    docs.sort();
    let mut out = vec![];
    for d in docs {
        for t in tells_in(&prose(&read(root, d)), tells) {
            out.push(format!("{d}: {t}"));
        }
    }
    out
}

/// A document's `<meta name=… content=…>`, decoded.
fn meta(body: &str, name: &str) -> Option<String> {
    let rx = Regex::new(&format!(
        r#"\x3Cmeta name="{}" content="([^"]+)""#,
        regex::escape(name)
    ))
    .expect("regex");
    rx.captures(body).map(|c| unescape(&c[1]))
}

/// Dashboard.Write.claimed_stage: the stage a document's own stepper marks current is Review (or Relecture).
pub fn at_review(body: &str) -> bool {
    let ol = re!(r#"(?s)\x3Col class="stages"[^>]*>(.*?)\x3C/ol>"#);
    let step = re!(r#"aria-current="step"[^>]*>\s*(\w+)"#);
    ol.captures(body)
        .and_then(|c| step.captures(&c[1]).map(|s| s[1].to_string()))
        .is_some_and(|l| l == "Review" || l == "Relecture")
}

/// Each document of private/docs at Review names the day it went to review and the one decision it needs, in
/// `crm:review-since` (YYYY-MM-DD) and `crm:decision`: the home page's « Docs to review » shows both (task comment-39ce1982).
pub fn review_metas(root: &Path, files: &[(String, String)]) -> Vec<String> {
    let day = re!(r"^\d{4}-\d\d-\d\d$");
    let mut docs = under(files, &["private/docs/"], &["html"]);
    docs.sort();
    let mut out = vec![];
    for d in docs {
        let text = read(root, d);
        if !at_review(&text) {
            continue;
        }
        let since = meta(&text, "crm:review-since").unwrap_or_default();
        let missing: Vec<&str> = [
            (!day.is_match(&since)).then_some("crm:review-since"),
            meta(&text, "crm:decision")
                .is_none()
                .then_some("crm:decision"),
        ]
        .into_iter()
        .flatten()
        .collect();
        if !missing.is_empty() {
            out.push(format!("{d} ({})", missing.join(", ")));
        }
    }
    out
}

/// The files that name a browser launch as data and start none (Dashboard.Lint's @browser_launch_own): the old lint itself,
/// load_acts and its test, the machine-budget measure's class rule, the harness's shared.ts (it takes its slot itself),
/// malt's profile.clj.
pub const BROWSER_LAUNCH_OWN: [&str; 6] = [
    "private/dashboard/lib/dashboard/lint.ex",
    "private/dashboard/test/dashboard/watch/load_acts_test.exs",
    "private/dashboard/lib/dashboard/watch/load_acts.ex",
    "private/docs/rfc-machine-budget-measure.exs",
    "private/replicator/harness/shared.ts",
    "private/malt/src/malt/profile.clj",
];

/// The browser slot rule on the whole tree (task browser-slots-everywhere): a file of `files` that starts a browser takes one
/// of the machine's slots through bin/browser-slot; [`BROWSER_LAUNCH_OWN`]'s paths are exempt.
pub fn browser_slots(root: &Path, files: &[(String, String)]) -> Vec<String> {
    let scan: Vec<(String, String)> = files
        .iter()
        .filter(|(_, p)| !BROWSER_LAUNCH_OWN.contains(&p.as_str()))
        .cloned()
        .collect();
    crate::browsers_without_slot(root, &scan)
        .into_iter()
        .map(|p| format!("{p}: starts a browser with no bin/browser-slot"))
        .collect()
}

/// Each of [`BROWSER_LAUNCH_OWN`]'s paths the repository no longer tracks: left listed, it would exempt a new file at that
/// path. lint_rules.rs's the_tree_passes_every_rule holds the list to the tree.
pub fn browser_launch_own_gone(tracked: &[(String, String)]) -> Vec<&'static str> {
    BROWSER_LAUNCH_OWN
        .into_iter()
        .filter(|own| !tracked.iter().any(|(_, p)| p == own))
        .collect()
}

/// ruff's flags, as Dashboard.Lint's `@ruff` (--force-exclude: ruff ignores its excludes for a named path otherwise).
pub const RUFF: [&str; 7] = [
    "uvx",
    "ruff",
    "check",
    "--isolated",
    "--force-exclude",
    "--select",
    "F,E9,B",
];
/// clj-kondo on what Clojure is left (the MCP servers').
pub const KONDO: [&str; 4] = ["clj-kondo", "--fail-level", "warning", "--lint"];

/// The tool runs for `paths` (each tracked, ours): ruff on the Python (a .py, or a #! naming python or `uv run`), deno on the JS and TS
/// (the replicator's apart, with no-sloppy-imports off: Vite resolves an import with no extension), clj-kondo on the
/// Clojure. Each an argv, run from the repository's top; none for a kind with no file.
pub fn tool_runs(root: &Path, paths: &[&str]) -> Vec<Vec<String>> {
    let python = |p: &str| {
        ext_in(p, &["py"])
            || (Path::new(p).extension().is_none()
                && read(root, p).lines().next().is_some_and(|l| {
                    l.starts_with("#!") && (l.contains("python") || l.contains("uv run"))
                }))
    };
    let py: Vec<&str> = paths.iter().copied().filter(|p| python(p)).collect();
    let js: Vec<&str> = paths
        .iter()
        .copied()
        .filter(|p| ext_in(p, &["js", "mjs", "ts", "tsx"]))
        .collect();
    let (rep, js): (Vec<&str>, Vec<&str>) = js
        .into_iter()
        .partition(|p| p.starts_with("private/replicator/"));
    let clj: Vec<&str> = paths
        .iter()
        .copied()
        .filter(|p| ext_in(p, &["clj", "cljc", "bb"]))
        .collect();
    let deno = |exclude: String, files: &[&str]| {
        let mut argv: Vec<String> = ["deno", "lint", "--quiet"].map(String::from).to_vec();
        argv.push(exclude);
        argv.extend(files.iter().map(|f| f.to_string()));
        argv
    };
    let mut runs = vec![];
    if !py.is_empty() {
        let mut argv: Vec<String> = RUFF.map(String::from).to_vec();
        argv.extend(py.iter().map(|f| f.to_string()));
        runs.push(argv);
    }
    if !rep.is_empty() {
        runs.push(deno(
            format!("{},no-sloppy-imports", crate::DENO_RULES_EXCLUDE),
            &rep,
        ));
    }
    if !js.is_empty() {
        runs.push(deno(crate::DENO_RULES_EXCLUDE.to_string(), &js));
    }
    if !clj.is_empty() {
        let mut argv: Vec<String> = KONDO.map(String::from).to_vec();
        argv.extend(clj.iter().map(|f| f.to_string()));
        runs.push(argv);
    }
    runs
}

/// One tool run from `root`: its findings, or why it did not run.
pub fn run_tool(root: &Path, argv: &[String]) -> Vec<String> {
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]).env("NO_COLOR", "1").current_dir(root);
    crate::findings(
        cmd,
        &format!("lint: {} needs it on PATH (bin/ci/lint-tools.sh)", argv[0]),
    )
}

/// The harness's warnings, then every rule but the tools on the whole tree, named: `(name, findings)`.
pub fn rules(
    root: &Path,
    files: &[(String, String)],
    tells: &[Tell],
) -> (Vec<String>, Vec<(&'static str, Vec<String>)>) {
    let (warnings, harness_errors) = harness(root);
    let found = vec![
        (
            "the harness must not know the orchestration or tasks",
            harness_errors,
        ),
        ("raw hex color (use a theme token)", hex_colors(root, files)),
        ("cite him as @nha, <date>", citations(root, files)),
        (
            "an AI tell in a document (the tells table)",
            ai_isms_docs(root, files, tells),
        ),
        (
            "a document at Review names its review day and its one decision (rfc skill, rule 18)",
            review_metas(root, files),
        ),
    ];
    (warnings, found)
}

/// A path whose change changes what a tool says of files it does not name (task precommit-fast's `lint_config?`): a tool's
/// config, or this gate. Staged, the tools then run on every tracked file of ours.
pub fn lint_config(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    [
        "ruff.toml",
        "pyproject.toml",
        ".shellcheckrc",
        "deno.json",
        "deno.jsonc",
    ]
    .contains(&name)
        || path.starts_with(".clj-kondo/")
        || path == "rust/crates/gate/src/lint.rs"
}

/// `dash lint`: every rule on the tree at `root`, then the browser slot rule and the tools on `staged` (the pre-commit's
/// paths; every tracked file of ours when one is a [`lint_config`]), or on every tracked file of ours with none. With none,
/// or when `deleted` (the commit's) names one, an exempt browser path no longer tracked fails. Ok with what it checked, else
/// each failing rule with its findings. The script ratchet and shellcheck are `dash scripts count --check`'s.
pub fn run(
    root: &Path,
    staged: Option<&[String]>,
    deleted: &[String],
    tells: &[Tell],
) -> Result<String, String> {
    let tracked = tracked(root)?;
    let mut failed: Vec<String> = vec![];
    let (warnings, found) = rules(root, &tracked, tells);
    let mut said = vec![];
    if !warnings.is_empty() {
        said.push(format!(
            "warning: the approved sandbox.sb names the orchestration, fix it in sandbox.sb.candidate for the next approval: {}",
            warnings.join(" ")
        ));
    }
    for (what, hits) in found {
        if !hits.is_empty() {
            failed.push(format!("{what}: {}", hits.join(" ")));
        }
    }
    let gone: Vec<&str> = browser_launch_own_gone(&tracked)
        .into_iter()
        .filter(|own| staged.is_none() || deleted.iter().any(|d| d == own))
        .collect();
    if !gone.is_empty() {
        failed.push(format!(
            "exempt in dash_gate::lint::BROWSER_LAUNCH_OWN and no longer tracked, remove its line: {}",
            gone.join(" ")
        ));
    }
    let scope: Vec<(String, String)> = match staged {
        Some(paths) if !paths.iter().any(|p| lint_config(p)) => tracked
            .iter()
            .filter(|(_, p)| paths.contains(p))
            .cloned()
            .collect(),
        _ => tracked.clone(),
    };
    let scope: Vec<(String, String)> = scope
        .into_iter()
        .filter(|(_, p)| crate::ratchet::ours(p))
        .collect();
    let text: Vec<(String, String)> = scope
        .iter()
        .filter(|(_, p)| !p.ends_with(".md"))
        .cloned()
        .collect();
    let slots = browser_slots(root, &text);
    if !slots.is_empty() {
        failed.push(slots.join("\n"));
    }
    let paths: Vec<&str> = scope.iter().map(|(_, p)| p.as_str()).collect();
    for argv in tool_runs(root, &paths) {
        let found = run_tool(root, &argv);
        if !found.is_empty() {
            failed.push(format!(
                "lint: {} failed:\n{}",
                argv[..2].join(" "),
                found.join("\n")
            ));
        }
    }
    match failed.is_empty() {
        true => {
            said.push(format!("lint ok: {} files", scope.len()));
            Ok(said.join("\n"))
        }
        false => Err([said, failed].concat().join("\n")),
    }
}
