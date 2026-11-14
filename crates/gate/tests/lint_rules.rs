//! `dash lint`'s rules (dash_gate::lint), each held to the cases of the Elixir `Dashboard.LintTest` it replaces (task
//! rust-lint-port), and the tree as it stands passing them all, so a port that reads more than its original fails here.
use dash_gate::lint::{self, Tell};
use dash_meter::TestDir;
use dash_meter::dash_test;
use regex::Regex;
use std::fs;
use std::path::Path;

/// `files` written under `root`, and the tracked list a rule reads, every file a plain one.
fn tree(root: &Path, files: &[(&str, &str)]) -> Vec<(String, String)> {
    for (path, text) in files {
        let at = root.join(path);
        fs::create_dir_all(at.parent().expect("parent")).expect("mkdir");
        fs::write(at, text).expect("write");
    }
    files
        .iter()
        .map(|(p, _)| ("100644".to_string(), p.to_string()))
        .collect()
}

#[dash_test]
fn the_harness_never_names_the_orchestration_and_the_approved_profile_only_warns() {
    let dir = TestDir::new("lint-harness");
    tree(
        &dir.0,
        &[
            ("Makefile", "build:\n\tcargo build\nrun-tasks:\n\ttrue\n"),
            ("sandbox/jobs.conf", "mail 5 dash mail poll\n"),
            (
                "sandbox.sb",
                "(allow file-read* (subpath \"/x/orchestrator\"))\n",
            ),
            (
                "sandbox.sb.candidate",
                "; the tasks folder\n(allow file-read*)\n",
            ),
        ],
    );
    let (warnings, strict) = lint::harness(&dir.0);
    // « tasks » fails in the Makefile; the candidate may say it, never the orchestration
    assert_eq!(strict, vec!["Makefile:3".to_string()]);
    assert_eq!(warnings, vec!["sandbox.sb:1".to_string()]);
    fs::write(
        dir.0.join("sandbox.sb.candidate"),
        "; ao session.sh delegate\n",
    )
    .expect("write");
    assert_eq!(
        lint::harness(&dir.0).1,
        ["Makefile:3", "sandbox.sb.candidate:1"]
    );
}

#[dash_test]
fn a_raw_hex_color_in_a_rust_page_fails_and_an_issue_number_does_not() {
    let dir = TestDir::new("lint-hex");
    let page = "rust/crates/mail/src/inbox.rs";
    // each color split, so this file holds none itself
    let text = concat!(
        "// fixes #127\nlet c = \"#f",
        "ff\";\nlet s = \"color: #3",
        "33\";\n// PR #1234567\n",
        // an anchor, a URL's anchor and a word's are no colors (/code-review of PR 1519)
        "a(href=\"#add\")\n// rfc-x.html#decade\n// abc#fff\n",
        // a format spec is no color either
        "format!(\"{:#010b}\", f)\n"
    );
    let files = tree(
        &dir.0,
        &[
            (page, text),
            (
                "rust/crates/surface/src/theme.rs",
                concat!("const INK: &str = \"#1a", "1a1a\";\n"),
            ),
            (
                "private/dashboard/lib/dashboard_web/x.ex",
                concat!("@c \"#ab", "c\"\n"),
            ),
        ],
    );
    assert_eq!(
        lint::hex_colors(&dir.0, &files),
        [format!("{page}:2"), format!("{page}:3")]
    );
}

#[dash_test]
fn his_words_are_cited_as_nha_in_rust_and_the_scripts() {
    let dir = TestDir::new("lint-citations");
    // spelled in two parts, so this file is no hit itself
    let old = format!("# ({}, 2026-10-05: « no »)\n", concat!("Nico", "las"));
    let files = tree(
        &dir.0,
        &[
            (
                "rust/crates/x/src/lib.rs",
                "// (@nha, 2026-10-05: « ok »)\n",
            ),
            ("bin/tool", &format!("#!/bin/sh\n{old}")),
            ("a.py", &old),
            ("notes.md", &old),
            ("private/dashboard/lib/x.ex", &old),
            ("bin/vendor/x.sh", &old),
        ],
    );
    assert_eq!(lint::citations(&dir.0, &files), ["bin/tool:2", "a.py:1"]);
}

fn em_dash() -> Vec<Tell> {
    vec![(
        Regex::new("(?i)—").expect("regex"),
        "an em-dash".into(),
        "a period".into(),
    )]
}

#[dash_test]
fn an_ai_tell_in_a_document_s_prose_fails_and_code_quotes_and_his_verbatim_words_pass() {
    let dir = TestDir::new("lint-isms");
    let doc = "private/docs/rfc-x.html";
    let files = tree(
        &dir.0,
        &[(
            doc,
            "<p>Plain.</p><code>a — b</code><pre>c — d</pre><p>« his — words »</p>\
             <blockquote data-verbatim>his — text</blockquote><p>He said &#8212; no.</p>",
        )],
    );
    assert_eq!(
        lint::ai_isms_docs(&dir.0, &files, &em_dash()),
        [format!("{doc}: « — »: an em-dash, write a period")]
    );
    fs::write(
        dir.0.join(doc),
        "<p>Plain.</p><svg><text>a — b</text></svg>",
    )
    .expect("write");
    assert!(
        lint::ai_isms_docs(&dir.0, &files, &em_dash()).is_empty(),
        "a diagram's text is never prose"
    );
    // a named entity is decoded too (/code-review of PR 1519)
    fs::write(dir.0.join(doc), "<p>A &mdash; b.</p>").expect("write");
    assert_eq!(
        lint::ai_isms_docs(&dir.0, &files, &em_dash()),
        [format!("{doc}: « — »: an em-dash, write a period")]
    );
    // the table is written for lowercase text with one apostrophe, as the outbound gate reads it
    let tell = vec![(
        Regex::new(r"it's not").expect("regex"),
        "a pivot".into(),
        "the point".into(),
    )];
    fs::write(dir.0.join(doc), "<p>It’s not that.</p>").expect("write");
    assert_eq!(
        lint::ai_isms_docs(&dir.0, &files, &tell),
        [format!("{doc}: « it's not »: a pivot, write the point")]
    );
}

#[dash_test]
fn a_document_at_review_names_its_day_and_its_decision() {
    let dir = TestDir::new("lint-review");
    let stages = r#"<ol class="stages"><li>Draft</li><li aria-current="step">Review</li></ol>"#;
    let files = tree(
        &dir.0,
        &[
            ("private/docs/a.html", stages),
            (
                "private/docs/b.html",
                &format!(
                    r#"<meta name="crm:review-since" content="2026-11-12"><meta name="crm:decision" content="Go?">{stages}"#
                ),
            ),
            (
                "private/docs/c.html",
                "<ol class=\"stages\"><li aria-current=\"step\">Draft</li></ol>",
            ),
        ],
    );
    assert_eq!(
        lint::review_metas(&dir.0, &files),
        ["private/docs/a.html (crm:review-since, crm:decision)"]
    );
}

#[dash_test]
fn a_browser_launch_takes_a_slot_and_an_exempt_path_gone_fails() {
    let dir = TestDir::new("lint-browser");
    let launch = concat!("chrome --head", "less --dump-dom x\n");
    let slot = concat!("bin/browser", "-slot run chrome --head", "less\n");
    let files = tree(
        &dir.0,
        &[("bin/a", launch), ("bin/b", slot), ("README.md", launch)],
    );
    let scan: Vec<_> = files
        .iter()
        .filter(|(_, p)| !p.ends_with(".md"))
        .cloned()
        .collect();
    assert_eq!(
        lint::browser_slots(&dir.0, &scan),
        ["bin/a: starts a browser with no bin/browser-slot"]
    );
    // the exempt paths: one no longer tracked is named, so it never exempts a new file there
    assert_eq!(
        lint::browser_launch_own_gone(&files),
        lint::BROWSER_LAUNCH_OWN
    );
    let own: Vec<_> = lint::BROWSER_LAUNCH_OWN
        .map(|p| ("100644".to_string(), p.to_string()))
        .to_vec();
    assert!(
        lint::browser_launch_own_gone(&own).is_empty(),
        "every exempt path tracked names none"
    );
}

/// Mix lint's @ruff_paths, its extensionless Python scripts, each found by its #! with no list: a renamed script leaves
/// nothing behind to fail on.
#[dash_test]
fn the_tools_find_each_kind_without_a_list() {
    let Some(top) = dash_meter::monorepo() else {
        println!("no monorepo beside rust/: the mirror");
        return;
    };
    let listed = [
        "private/dashboard/priv/commands/klar-mail",
        "private/dashboard/priv/commands/comments-anchored",
        "private/dashboard/test/scripts/offline-check",
    ];
    let paths = [
        &listed[..],
        &[
            "x.py",
            "private/replicator/src/a.ts",
            "assets/b.js",
            "private/malt/src/c.clj",
            "README.md",
        ],
    ]
    .concat();
    let present: Vec<&str> = paths
        .iter()
        .copied()
        .filter(|p| !listed.contains(p) || top.join(p).is_file())
        .collect();
    let runs = lint::tool_runs(&top, &present);
    let ruff = runs.iter().find(|r| r[1] == "ruff").expect("a ruff run");
    for p in present.iter().filter(|p| listed.contains(p)) {
        assert!(ruff.contains(&p.to_string()), "{p} is Python by its #!");
    }
    assert!(ruff.contains(&"x.py".to_string()));
    let denos: Vec<&Vec<String>> = runs.iter().filter(|r| r[0] == "deno").collect();
    assert_eq!(denos.len(), 2, "the replicator's files apart: {runs:?}");
    assert!(
        denos[0]
            .last()
            .is_some_and(|f| f == "private/replicator/src/a.ts")
    );
    assert!(denos[0][3].ends_with(",no-sloppy-imports"));
    assert!(
        runs.iter()
            .any(|r| r[0] == "clj-kondo" && r.last().is_some_and(|f| f.ends_with("c.clj")))
    );
    assert!(!runs.concat().contains(&"README.md".to_string()));
    assert!(!dash_gate::ratchet::ours(
        "private/dashboard/assets/vendor/x.js"
    ));
    assert!(!dash_gate::ratchet::ours(
        ".claude/skills/a/references/x.py"
    ));
}

/// The tree as it stands passes every rule: a port that reads more than its original would fail the next commit.
#[dash_test]
fn the_tree_passes_every_rule() {
    let Some(top) = dash_meter::monorepo() else {
        println!("no monorepo beside rust/: the mirror");
        return;
    };
    let index = dash_gate::index(&top).expect("index");
    let files = dash_gate::on_disk(&top, &index);
    let failed: Vec<_> = lint::rules(&top, &index, &files, &[])
        .1
        .into_iter()
        .filter(|(_, hits)| !hits.is_empty())
        .collect();
    assert!(
        failed.is_empty(),
        "fix each finding, or the rule that reads more than its original. Found: {failed:#?}"
    );
    assert_eq!(
        lint::browser_launch_own_gone(&index),
        Vec::<&str>::new(),
        "remove each from BROWSER_LAUNCH_OWN"
    );
}

/// Each job of a workflow, `(« file job », its text)`, split at its two-space key as ci_checkout_test.exs splits them.
fn jobs(dir: &Path) -> Vec<(String, String)> {
    let mut out = vec![];
    let mut names: Vec<_> = fs::read_dir(dir)
        .expect("workflows")
        .map(|e| e.expect("entry").path())
        .collect();
    names.sort();
    for path in names
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "yml"))
    {
        let text = fs::read_to_string(&path).expect("workflow");
        let file = path
            .file_name()
            .expect("name")
            .to_string_lossy()
            .into_owned();
        let Some((_, body)) = text.split_once("\njobs:\n") else {
            continue;
        };
        let mut job: Option<(String, String)> = None;
        for line in body.lines() {
            let key = line.strip_prefix("  ").and_then(|l| l.strip_suffix(':'));
            match key.filter(|k| !k.starts_with([' ', '#']) && !k.contains(' ')) {
                Some(name) => out.extend(job.replace((format!("{file} {name}"), String::new()))),
                None => {
                    if let Some((_, t)) = job.as_mut() {
                        t.push_str(line);
                        t.push('\n');
                    }
                }
            }
        }
        out.extend(job);
    }
    out
}

/// ci_router_test.exs's « a hosted job checks out code only », left out by name in ported-tests.txt (no-elixir freezes its
/// assertion that check.yml runs mix lint): its rules, held here on the workflows' text. A hosted job checks out code only,
/// no blob of private/ but the dashboard's and Jev's named files, none of to_process/, with no token on origin; the four
/// routed jobs read dashboard.yml's pick; check.yml's lint runs dash lint, built from rust/, never mix lint (task
/// rust-lint-port), and private.yml's runs it whatever the step before said.
#[dash_test]
fn a_hosted_job_checks_out_code_only_and_the_lint_job_runs_dash_lint() {
    let Some(top) = dash_meter::monorepo() else {
        println!("no monorepo beside rust/: the mirror");
        return;
    };
    let all = jobs(&top.join(".github/workflows"));
    let runs_on = |t: &str| {
        t.lines()
            .find(|l| l.starts_with("    runs-on:"))
            .unwrap_or_default()
            .to_string()
    };
    let hosted: Vec<&(String, String)> = all
        .iter()
        .filter(|(_, t)| {
            let r = runs_on(t);
            r.contains("outputs.route)['runs-on']")
                || r.contains("ubuntu")
                || r.contains("fromJSON(inputs.runner)")
        })
        .collect();
    let names: Vec<&str> = hosted.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "check.yml lint",
            "check.yml live-data",
            "ci-probe.yml probe",
            "ci-usage.yml read",
            "dashboard.yml check",
        ]
    );
    let code = [
        "/*",
        "!/private/",
        "!/to_process/",
        "/private/docs/shell/",
        "/private/ats/context-questions.json",
        "/private/ats/decision-rubric.json",
        "/private/ats/density-rubric.json",
        "/private/ats/doc-questions.json",
        "/private/ats/draft-questions.json",
        "/private/ats/questions.json",
    ];
    let allowed = [
        "/site/",
        "/fly.toml",
        "/bin/site-check",
        "/bin/site-start",
        "/rust/",
        "/bin/ci/site-host.sh",
    ];
    let clojure = [
        "/private/malt/src/",
        "/private/whatsapp/src/",
        "/private/whatsapp/test/",
    ];
    for (name, text) in &hosted {
        let checkouts: Vec<&str> = text
            .split("\n      - ")
            .filter(|s| s.contains("uses: actions/checkout@"))
            .collect();
        if ["ci-usage.yml read", "ci-probe.yml probe"].contains(&name.as_str()) {
            assert!(
                checkouts.is_empty(),
                "{name} checks nothing out of the repository"
            );
            continue;
        }
        // dashboard.yml's full checkout serves this Mac only
        let sparse: Vec<&&str> = checkouts
            .iter()
            .filter(|s| !s.contains("if: env.HOSTED != 'true'"))
            .collect();
        assert!(!sparse.is_empty(), "{name}");
        for s in sparse {
            for held in [
                "filter: blob:none",
                "persist-credentials: false",
                "sparse-checkout-cone-mode: false",
            ] {
                assert!(s.contains(held), "{name}: no {held}");
            }
            assert!(
                !s.contains("fetch-depth: 0"),
                "{name}: a fetch-depth 0 fetches the store branch"
            );
            let patterns: Vec<&str> = s
                .split_once("sparse-checkout: |")
                .map(|(_, l)| {
                    l.lines()
                        .skip(1)
                        .map(str::trim)
                        .take_while(|l| !l.is_empty() && !l.starts_with('#') && !l.contains(':'))
                        .collect()
                })
                .unwrap_or_default();
            for p in &patterns {
                let ok = code.contains(p)
                    || allowed.contains(p)
                    || p.starts_with("/private/dashboard/")
                    || (name == "check.yml lint" && clojure.contains(p));
                assert!(ok, "{name}: {p} in its checkout");
            }
            if patterns.contains(&"/*") {
                assert_eq!(patterns[..3], code[..3], "{name}: /* then both exclusions");
            }
        }
    }
    // the router's pick, character for character, on the four routed jobs
    let job = |n: &str| &all.iter().find(|(name, _)| name == n).expect(n).1;
    let pick = |t: &str| {
        (
            runs_on(t),
            t.lines()
                .find(|l| l.trim_start().starts_with("HOSTED:"))
                .unwrap_or_default()
                .to_string(),
        )
    };
    let check = pick(job("dashboard.yml check"));
    assert!(check.0.contains(
        "needs.define-ci.result == 'success' && fromJSON(needs.define-ci.outputs.route)['runs-on']"
    ));
    // site.yml's test runs on our VPS since task rust-ci-off-mac, routed no more
    for n in ["check.yml lint", "check.yml live-data"] {
        assert!(job(n).contains("needs: define-ci"), "{n}");
        assert_eq!(pick(job(n)), check, "{n} reads dashboard.yml's pick");
    }
    let lint = job("check.yml lint");
    assert!(lint.contains("if: ${{ !cancelled() && github.event_name != 'pull_request' }}"));
    assert!(
        lint.contains("uses: ./.github/actions/main-bin\n        with: {name: dash}")
            && lint.contains("run: '{ $RUNNER_TEMP/dash lint || { [ $? = 2 ] && cd rust && cargo run -q --locked -p dash -- lint; }; } 2>&1")
            && lint.contains("- if: env.HOSTED != 'true'\n        run: cd rust && cargo run -q --locked -p dash -- lint 2>&1"),
        "check.yml's lint runs main's dash lint on a hosted machine, this checkout's on the Mac"
    );
    assert!(lint.contains("- if: env.HOSTED == 'true'\n        run: bin/ci/lint-tools.sh install"));
    let private = job("private.yml private");
    assert!(private.contains(
        "- if: success() || failure()\n        run: bin/ci/lint-tools.sh && cd rust && cargo run -q --locked -p dash -- lint 2>&1"
    ));
    for (name, text) in &all {
        let runs: Vec<&str> = text
            .lines()
            .filter(|l| l.trim_start().starts_with("run:") || l.trim_start().starts_with("- run:"))
            .collect();
        assert!(
            !runs
                .iter()
                .any(|r| r.contains("mix lint ") || r.ends_with("mix lint")),
            "{name} runs mix lint"
        );
        if name == "check.yml lint" {
            assert!(
                !runs.iter().any(|r| r.contains(".check")),
                "his data's gates are private.yml's"
            );
        }
    }
    let tools = fs::read_to_string(top.join("bin/ci/lint-tools.sh")).expect("lint-tools.sh");
    assert!(tools.contains(
        "apt-get -o Acquire::Retries=5 install -y --no-install-recommends poppler-utils"
    ));
    assert_eq!(tools.matches("sha256sum -c -").count(), 1);
    let sums = tools
        .lines()
        .filter(|l| l.trim().len() == 64 && l.trim().chars().all(|c| c.is_ascii_hexdigit()))
        .count();
    assert_eq!(sums, 3, "each release binary against its sum");
}

/// task rust-lint-scripts-port: dash lint shellchecks the shell scripts on disk, found by name or line 1 with no list, as
/// mix lint did in CI until task rust-lint-port took it out; a zsh, Python or prose file never. The full run reports a
/// finding, through a stand-in shellcheck (CI's test machines may have none).
#[dash_test]
fn shellcheck_runs_on_the_shell_scripts_by_name_and_line_one() {
    let dir = TestDir::new("lint-shellcheck");
    let files = [
        ("a.sh", "echo BAD\n"),
        ("b.bash", "echo b\n"),
        ("bin/deploy.d", "#!/usr/bin/env bash\necho d\n"),
        ("bin/hook", "#!/usr/bin/env bash\necho ok\n"),
        ("bin/lib", "# shellcheck shell=bash disable=SC2034\nx=1\n"),
        ("bin/tool", "#!/usr/bin/env python3\nprint(1)\n"),
        ("bin/z", "#!/bin/zsh\necho z\n"),
        ("c.zsh", "#!/bin/bash\necho c\n"),
        ("lib/common.inc", "# shellcheck shell=sh\nx=1\n"),
        ("notes.md", "# notes\n"),
    ];
    tree(&dir.0, &files);
    let paths: Vec<&str> = files.iter().map(|(p, _)| *p).collect();
    let runs = lint::tool_runs(&dir.0, &paths);
    let sc: Vec<&Vec<String>> = runs.iter().filter(|r| r[0] == "shellcheck").collect();
    assert_eq!(sc.len(), 1, "{runs:?}");
    assert_eq!(
        sc[0][..],
        [
            "shellcheck",
            "--exclude=SC1091",
            "a.sh",
            "b.bash",
            "bin/deploy.d",
            "bin/hook",
            "bin/lib",
            "lib/common.inc"
        ]
    );
    // no shell script, no run
    assert!(
        lint::tool_runs(&dir.0, &["c.zsh", "notes.md"])
            .iter()
            .all(|r| r[0] != "shellcheck")
    );
    // the full run: a stand-in outside the repository fails on BAD, as shellcheck on a finding
    let bin = TestDir::new("lint-shellcheck-bin");
    let fake = bin.0.join("shellcheck");
    fs::write(
        &fake,
        "#!/bin/sh\n[ \"$1\" = --exclude=SC1091 ] || { echo \"args: $*\"; exit 2; }\nshift\n! grep -n BAD \"$@\"\n",
    )
    .expect("stand-in");
    let mut mode = fs::metadata(&fake).expect("stand-in").permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut mode, 0o755);
    fs::set_permissions(&fake, mode).expect("chmod");
    // each test is a process of its own under nextest (rust/CLAUDE.md rule 1), so no other thread reads the env; the
    // fixture repository's own index, never a pre-commit's
    unsafe {
        for v in dash_gate::merge::GIT_HOOK_VARS {
            std::env::remove_var(v);
        }
        let path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{path}", bin.0.display()));
    }
    fs::remove_file(dir.0.join("bin/tool")).expect("no ruff run");
    let git = |args: &[&str]| {
        let ok = std::process::Command::new("git")
            .args(args)
            .current_dir(&dir.0)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .status()
            .expect("git")
            .success();
        assert!(ok, "git {args:?}");
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    let said = lint::run(&dir.0, None, &[], &[]).expect_err("a finding");
    assert!(
        said.contains("lint: shellcheck --exclude=SC1091 failed:\na.sh:1:echo BAD"),
        "{said}"
    );
}

/// /code-review of PR rust-lint-port: each narrowing the first port missed, held to its Elixir original.
#[dash_test]
fn the_review_s_cases_hold() {
    let dir = TestDir::new("lint-review-cases");
    // a uv script is Python
    tree(
        &dir.0,
        &[(
            "bin/tool",
            "#!/usr/bin/env -S uv run --quiet --script\nprint(1)\n",
        )],
    );
    let runs = lint::tool_runs(&dir.0, &["bin/tool"]);
    assert!(
        runs.iter()
            .any(|r| r[1] == "ruff" && r.last().is_some_and(|p| p == "bin/tool")),
        "{runs:?}"
    );
    // a tool's config sends the staged lint to every file
    for config in [
        ".clj-kondo/config.edn",
        "deno.jsonc",
        "rust/crates/gate/src/lint.rs",
        "rust/crates/gate/src/tool_flags.rs",
    ] {
        assert!(lint::lint_config(config), "{config}");
    }
    assert!(!lint::lint_config("rust/crates/gate/src/lib.rs"));
    // ruff runs --isolated: its config changes nothing
    assert!(!lint::lint_config("a/pyproject.toml"));
}

/// `dash lint` on a repository: an exempt browser path gone fails the full run, and a commit only when it deletes it.
#[dash_test]
fn an_exempt_browser_path_gone_fails_the_full_run_and_the_commit_deleting_it() {
    // SAFETY: before any thread; the fixture repository's own index, never a pre-commit's (see his_live_data_...)
    unsafe {
        for v in dash_gate::merge::GIT_HOOK_VARS {
            std::env::remove_var(v);
        }
    }
    let dir = TestDir::new("lint-run");
    tree(
        &dir.0,
        &[(".gitignore", "private/live/\n"), ("notes.txt", "one\n")],
    );
    let git = |args: &[&str]| {
        let ok = std::process::Command::new("git")
            .args(args)
            .current_dir(&dir.0)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .status()
            .expect("git")
            .success();
        assert!(ok, "git {args:?}");
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    let staged = ["notes.txt".to_string()];
    let said = lint::run(&dir.0, Some(&staged), &[], &[]);
    assert_eq!(said.as_deref(), Ok("lint ok: 1 files"));
    let gone = lint::BROWSER_LAUNCH_OWN[2].to_string();
    let said = lint::run(&dir.0, Some(&staged), std::slice::from_ref(&gone), &[])
        .expect_err("a deleted exempt path");
    assert!(said.contains(&format!("remove its line: {gone}")), "{said}");
    let said = lint::run(&dir.0, None, &[], &[]).expect_err("the full run");
    assert!(
        lint::BROWSER_LAUNCH_OWN
            .iter()
            .all(|own| said.contains(own)),
        "{said}"
    );
    // a hosted sparse checkout: tracked and not on disk is not gone (/code-review of PR 1519)
    let own: Vec<(&str, &str)> = lint::BROWSER_LAUNCH_OWN
        .iter()
        .map(|p| (*p, "x\n"))
        .collect();
    tree(&dir.0, &own);
    git(&["add", "."]);
    for p in lint::BROWSER_LAUNCH_OWN {
        fs::remove_file(dir.0.join(p)).expect("rm");
    }
    let said = lint::run(&dir.0, None, &[], &[]);
    assert!(said.is_ok(), "{said:?}");
    // his live data (live-data-test --check, ported): tracked, or no longer ignored, fails
    tree(&dir.0, &[("private/live/queue.edn", "[]\n")]);
    git(&["add", "-f", "private/live/queue.edn"]);
    let said = lint::run(&dir.0, None, &[], &[]).expect_err("live data tracked");
    assert!(said.contains("private/live/queue.edn is tracked"), "{said}");
    git(&["rm", "-q", "--cached", "private/live/queue.edn"]);
    tree(&dir.0, &[(".gitignore", "\n")]);
    let said = lint::run(&dir.0, None, &[], &[]).expect_err("live data not ignored");
    assert!(
        said.contains("private/live/comments.edn is not ignored"),
        "{said}"
    );
}
