//! `dash sandbox gate <dir>` (dash_gate::merge), each case of the Elixir `Dashboard.MergeGateTest` it replaces, and the one it
//! fixes (task rust-merge-gate-port): PR 311's merge fails, naming each file and line and what the approved copy lacks, until 311
//! is the approved copy; host/.merged, which the approved host_copy writes at run time, is the copy's.
use dash_gate::merge::{self, approved_half, graal_lacks};
use dash_meter::TestDir;
use dash_meter::memory::TestBudget;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// the fixture's own repository, never the one a pre-commit running this test has in its variables
fn git(dir: &Path, args: &[&str]) -> String {
    let mut git = Command::new("git");
    for v in merge::GIT_HOOK_VARS {
        git.env_remove(v);
    }
    let out = git
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write(dir: &Path, files: &[(&str, &str)]) {
    for (f, body) in files {
        let p = dir.join(f);
        fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
        fs::write(p, body).expect("write");
    }
}

fn commit(dir: &Path, files: &[(&str, &str)]) -> String {
    write(dir, files);
    git(dir, &["add", "-A"]);
    git(
        dir,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "c",
        ],
    );
    git(dir, &["rev-parse", "HEAD"])
}

fn approve(main: &Path, sha: &str) {
    write(main, &[(".claude/run/approved", &format!("{sha}\n"))]);
}

/// A sandbox-apply with its lists and one case label per verb, and host_copy writing its .merged as bin/sandbox-apply does,
/// a log beside it on stderr, its closing brace followed by a comment, and mirror_init's other $tmp after it.
fn kernel(set: &str, verbs: &[&str]) -> String {
    let labels: String = verbs
        .iter()
        .map(|v| format!("  {v})\n    exit 0 ;;\n"))
        .collect();
    format!(
        "#!/usr/bin/env bash\nSET=({set})\nMERGED=(Makefile bin/crm-phone bin/ci-runner)\n\
         host_copy() {{\n  tmp=$(mktemp -d)\n  git archive x 2>\"$tmp/archive.log\" && rm \"$tmp/archive.log\"\n\
         \x20     echo \"$m\" > \"$tmp/.merged\"; }}\n  mv \"$tmp\" \"$dir\"\n}} # host_copy\n\
         mirror_init() {{\n  git init -q --bare \"$tmp\" && echo x > \"$tmp/objects/info/alternates\"\n}}\n\
         case \"${{1:-}}\" in\n{labels}esac\n"
    )
}

struct Repo {
    _root: TestDir,
    main: PathBuf,
    wt: PathBuf,
    a: String,
    s: String,
}

/// main before PR 311 (no launch verb, no jobs.conf), then 311's merge: its own lines, and prose, a comment and docs that name
/// the verb too; a worktree on it
fn repo(name: &str) -> Repo {
    let root = TestDir::new(name);
    let main = root.0.join("main");
    fs::create_dir_all(&main).expect("main");
    git(&main, &["init", "-q", "-b", "master"]);
    let a = commit(
        &main,
        &[
            (
                "bin/sandbox-apply",
                &kernel(
                    "sandbox.sb sandbox/flags.sh bin/sandbox-apply",
                    &["--check", "sync", "approve"],
                ),
            ),
            ("sandbox.sb", "(version 1)\n"),
            ("sandbox/flags.sh", "flags\n"),
            ("sandbox/runner.sh", "runner\n"),
            (
                "Makefile",
                "claude:\n\t@\"$(CURDIR)/bin/sandbox-apply\" --check\n",
            ),
            ("bin/crm-phone", "\"$host/bin/sandbox-apply\" sync\n"),
            (
                "bin/ci-runner",
                "launcher=$(cd \"$conf/host/sandbox\" && pwd -P)/runner.sh\n",
            ),
            ("herdr/session.sh", "echo make claude\n"),
        ],
    );
    let s = commit(
        &main,
        &[
            (
                "bin/sandbox-apply",
                &kernel(
                    "sandbox.sb sandbox/jobs.conf bin/sandbox-apply",
                    &["--check", "sync", "approve", "launch"],
                ),
            ),
            ("sandbox/jobs.conf", "claude\n"),
            (
                "Makefile",
                "# Every launch is the approved copy's bin/sandbox-apply launch <job>\n\
                 LAUNCH = { test -e \"$(CONF)/host\" || \"$(CURDIR)/bin/sandbox-apply\" --init; } && exec \"$(CONF)/host/bin/sandbox-apply\" launch\n",
            ),
            (
                "bin/crm-phone",
                "  <array><string>$host/bin/sandbox-apply</string><string>launch</string><string>dashboard</string></array>\n",
            ),
            (
                "bin/ci-runner",
                "launcher=$(cd \"$conf/host/sandbox\" && pwd -P)/runner.sh\n\
                 echo \"$(date +%FT%T) ci-runner: runner at $root, inside its job of $conf/host/sandbox/jobs.conf\"\n\
                 echo \"the approved copy's sandbox-apply has no launch yet\"\n",
            ),
            (
                "herdr/session.sh",
                "  if [ \"$repo\" = site_perso ]; then echo \"$p$(printf %q \"${SITE_PERSO_CONF:-$HOME/.config/site_perso}/host/bin/sandbox-apply\") launch claude${1:+ $1}\"\n",
            ),
            (
                "private/dashboard/lib/handoff.ex",
                "  @doc \"bin/sandbox-apply launch claude\"\n",
            ),
            (
                "private/store/tasks.edn",
                ":why \"host/bin/sandbox-apply launch, which the approved copy lacks\"\n",
            ),
            (
                "bin/comment-wake",
                "\t@# host/bin/sandbox-apply launch claude\nsync  # then host/sandbox/jobs.conf, sandbox-apply launch\n",
            ),
            ("EXPLAINER.md", "bin/sandbox-apply launch claude\n"),
            (
                "bin/approved-test",
                "\"$r/bin/sandbox-apply\" launch claude\n",
            ),
        ],
    );
    let wt = root.0.join("wt");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            wt.to_str().expect("utf-8"),
            &s,
        ],
    );
    Repo {
        _root: root,
        main,
        wt,
        a,
        s,
    }
}

/// the lines between the verdict's first and last: what it names
fn hits(out: &str) -> Vec<&str> {
    let l: Vec<&str> = out.split('\n').collect();
    l[1..l.len() - 1].to_vec()
}

#[test]
fn pr_311_s_merge_fails_before_his_tap_naming_each_file_and_line() {
    let _budget = TestBudget::new();
    let r = repo("gate-311");
    approve(&r.main, &r.a);
    let out = merge::run(&r.wt, None).expect_err("refused");
    assert!(
        out.contains(&format!("approved copy ({}) lacks", &r.a[..12])),
        "{out}"
    );
    assert_eq!(
        hits(&out),
        [
            "Makefile:2: sandbox-apply launch",
            "bin/ci-runner:2: host/sandbox/jobs.conf",
            // a log line's prose: in a host-copy file, any word after sandbox-apply its script lacks is a call
            "bin/ci-runner:3: sandbox-apply has",
            "herdr/session.sh:1: sandbox-apply launch",
        ]
    );
    assert!(out.ends_with("Merge the sandbox change alone, let him tap it, then the callers (PR 317 reverted PR 311 for this)"));
}

/// bin/crm-phone:207 (`build_dash`) reads host/.merged, which host_copy writes into the copy at run time and no commit holds: the
/// Elixir gate refused every PR for it (task rust-merge-gate-port). It holds only while the approved host_copy writes it.
#[test]
fn host_merged_the_approved_host_copy_writes_is_present_crm_phone_207() {
    let _budget = TestBudget::new();
    let r = repo("gate-merged");
    approve(&r.main, &r.s);
    // bin/crm-phone:207, verbatim
    let line = "  m=$(cat \"$host/.merged\" 2>/dev/null) || return 1\n";
    // mirror_init's $tmp, past host_copy's « } # host_copy », is no copy; nor is a log host_copy writes on stderr
    let mirror = "cat \"$host/objects/info/alternates\" \"$host/archive.log\"\n";
    commit(
        &r.wt,
        &[
            ("bin/crm-phone", line),
            ("bin/ci-runner", &format!("x\n{line}{mirror}")),
        ],
    );
    let out = merge::run(&r.wt, None).expect_err("refused");
    assert_eq!(
        hits(&out),
        [
            "bin/ci-runner:3: host/objects/info/alternates",
            "bin/ci-runner:3: host/archive.log"
        ]
    );

    // an approved script whose host_copy writes no .merged: the read is a hit again
    let no_write = kernel(
        "sandbox.sb sandbox/jobs.conf bin/sandbox-apply",
        &["--check", "sync", "approve", "launch"],
    )
    .replace("echo \"$m\" > \"$tmp/.merged\"", "true");
    let a = commit(&r.main, &[("bin/sandbox-apply", &no_write)]);
    approve(&r.main, &a);
    let out = merge::run(&r.wt, None).expect_err("refused");
    assert_eq!(
        hits(&out),
        [
            "bin/ci-runner:2: host/.merged",
            "bin/ci-runner:3: host/objects/info/alternates",
            "bin/ci-runner:3: host/archive.log",
            "bin/crm-phone:1: host/.merged",
        ]
    );
}

// bin/crm-phone's launch above is no hit: a MERGED file runs from the host copy, which sync leaves as it is while main holds a
// sandbox-apply he has not tapped. The Makefile also runs from the working tree: only its ON_APPROVED_COPY half is the copy's.
#[test]
fn a_merged_script_and_the_makefile_s_approved_half_call_a_new_verb_ahead_of_his_tap() {
    let _budget = TestBudget::new();
    let r = repo("gate-half");
    approve(&r.main, &r.a);
    commit(
        &r.wt,
        &[
            (
                "Makefile",
                "ifdef ON_APPROVED_COPY\n  LAUNCH = exec \"$(HERE)/bin/sandbox-apply\" launch\nendif\nclaude:\n\
                 ifndef ON_APPROVED_COPY\n\t@\"$(CURDIR)/bin/sandbox-apply\" launch claude\nelse\n\
                 \t@\"$(HERE)/bin/sandbox-apply\" launch claude\nendif\n",
            ),
            ("herdr/session.sh", "echo make claude\n"),
            (
                "bin/ci-runner",
                "exec \"$conf/host/bin/sandbox-apply\" launch ci-runner\n",
            ),
        ],
    );
    let out = merge::run(&r.wt, None).expect_err("refused");
    assert_eq!(hits(&out), ["Makefile:6: sandbox-apply launch"]);
}

// A refused tap, then a PR reverting its script alone: sync takes main's MERGED files again, so a call they keep to the gone verb
// is a hit (/code-review).
#[test]
fn a_merged_call_to_a_verb_the_pr_s_own_script_lacks_is_a_hit() {
    let _budget = TestBudget::new();
    let r = repo("gate-revert");
    approve(&r.main, &r.a);
    commit(
        &r.wt,
        &[
            (
                "bin/sandbox-apply",
                &kernel(
                    "sandbox.sb bin/sandbox-apply",
                    &["--check", "sync", "approve"],
                ),
            ),
            (
                "bin/ci-runner",
                "exec \"$conf/host/bin/sandbox-apply\" launch ci-runner\n",
            ),
            (
                "Makefile",
                "claude:\n\t@\"$(CURDIR)/bin/sandbox-apply\" --check\n",
            ),
            ("herdr/session.sh", "echo make claude\n"),
            ("bin/crm-phone", "\"$host/bin/sandbox-apply\" sync\n"),
        ],
    );
    let out = merge::run(&r.wt, None).expect_err("refused");
    assert!(
        out.contains("bin/ci-runner:1: sandbox-apply launch"),
        "{out}"
    );
}

#[test]
fn approved_half_follows_nested_and_else_branches() {
    let _budget = TestBudget::new();
    let text = "a\nifndef ON_APPROVED_COPY\nb\nelse\nifeq (x,y)\nc\nendif\nd\nendif\ne\nifdef ON_APPROVED_COPY\nf\nendif\n";
    assert_eq!(
        approved_half(text).into_iter().collect::<Vec<_>>(),
        [4, 5, 6, 7, 8, 11, 12]
    );
}

#[test]
fn approved_half_reads_a_recipe_s_shell_if_and_else_as_recipe_text() {
    let _budget = TestBudget::new();
    let text = "ifdef ON_APPROVED_COPY\nt:\n\tif [ x ]; then\n\telse\n\tfi\nelse\nu\nendif\nv\n";
    assert_eq!(
        approved_half(text).into_iter().collect::<Vec<_>>(),
        [1, 2, 3, 4, 5]
    );
}

#[test]
fn it_passes_once_311_is_the_approved_copy() {
    let _budget = TestBudget::new();
    let r = repo("gate-pass");
    approve(&r.main, &r.s);
    let ok = merge::run(&r.wt, None).expect("OK");
    assert!(ok.starts_with("merge gate OK: "), "{ok}");
}

#[test]
fn the_main_checkout_is_not_judged_no_approval_or_no_checkout_is_never_ok() {
    let _budget = TestBudget::new();
    let r = repo("gate-main");
    approve(&r.main, &r.a);
    let said = merge::run(&r.main, None).expect("not judged");
    assert!(
        said.starts_with("merge gate: ") && said.contains("is the main checkout"),
        "{said}"
    );

    approve(&r.main, "abc123");
    let why = merge::run(&r.wt, None).expect_err("no sha");
    assert!(
        why.starts_with("merge gate: cannot judge: ") && why.contains("holds no full sha"),
        "{why}"
    );

    fs::remove_file(r.main.join(".claude/run/approved")).expect("rm");
    let why = merge::run(&r.wt, None).expect_err("no record");
    assert!(
        why.starts_with("merge gate: cannot judge: no approved commit on record"),
        "{why}"
    );

    let plain = r.main.parent().expect("root").join("plain");
    fs::create_dir_all(&plain).expect("plain");
    let why = merge::run(&plain, None).expect_err("no checkout");
    assert!(
        why.starts_with("merge gate: cannot judge: git rev-parse"),
        "{why}"
    );
    let why = merge::run(&plain.with_file_name("plain-typo"), None).expect_err("no folder");
    assert!(why.starts_with("merge gate: cannot judge: "), "{why}");
}

#[test]
fn a_verb_in_a_label_of_several_and_a_priv_commands_caller_are_seen() {
    let _budget = TestBudget::new();
    let r = repo("gate-labels");
    approve(&r.main, &r.a);
    commit(
        &r.wt,
        &[
            (
                "bin/sandbox-apply",
                &kernel(
                    "sandbox.sb sandbox/jobs.conf bin/sandbox-apply",
                    &["--check", "sync", "approve", "launch | run"],
                ),
            ),
            (
                "private/dashboard/priv/commands/w",
                "\"$h/bin/sandbox-apply\" run x\n",
            ),
        ],
    );
    let out = merge::run(&r.wt, None).expect_err("refused");
    assert!(
        out.contains("private/dashboard/priv/commands/w:1: sandbox-apply run"),
        "{out}"
    );
    assert!(
        out.contains("herdr/session.sh:1: sandbox-apply launch"),
        "{out}"
    );
}

/// A Rust crate's `tests/` file is test data: its fixtures name host files no merge calls (main failed its own gate on this
/// file's fixtures, review-lane, 2026-11-12). The same line in a crate's src/, or in any other `tests/` folder, is a call.
#[test]
fn a_rust_tests_folder_is_test_data_never_a_call() {
    let _budget = TestBudget::new();
    let r = repo("gate-rust-tests");
    approve(&r.main, &r.s);
    let fixture = "let mirror = \"cat \\\"$host/archive.log\\\"\";\n";
    commit(&r.wt, &[("rust/crates/gate/tests/merge_gate.rs", fixture)]);
    let ok = merge::run(&r.wt, None).expect("a test's fixture is no call");
    assert!(ok.starts_with("merge gate OK"), "{ok}");
    commit(
        &r.wt,
        &[
            ("rust/crates/gate/src/x.rs", fixture),
            ("bin/tests/run", fixture),
        ],
    );
    let out = merge::run(&r.wt, None).expect_err("refused");
    assert_eq!(
        hits(&out),
        [
            "bin/tests/run:1: host/archive.log",
            "rust/crates/gate/src/x.rs:1: host/archive.log"
        ]
    );
}

#[test]
fn an_approved_commit_it_cannot_read_fails_it_never_ok() {
    let _budget = TestBudget::new();
    let r = repo("gate-unread");
    approve(&r.main, &"0".repeat(40));
    let why = merge::run(&r.wt, None).expect_err("unread");
    assert!(
        why.starts_with("merge gate: cannot judge: git show "),
        "{why}"
    );
}

#[test]
fn an_approved_script_whose_set_it_cannot_read_fails_it_never_ok() {
    let _budget = TestBudget::new();
    let r = repo("gate-set");
    let a = commit(
        &r.main,
        &[(
            "bin/sandbox-apply",
            "#!/usr/bin/env bash\nSET=(\n  sandbox.sb\n)\n",
        )],
    );
    approve(&r.main, &a);
    let why = merge::run(&r.wt, None).expect_err("no SET");
    assert!(
        why.starts_with("merge gate: cannot judge: no one-line SET=( "),
        "{why}"
    );
}

const GRAAL_LAUNCH: &str = "SET=(.devcontainer/flare-sandbox.sb scripts/sandbox/)\n";

/// only a read at the approved sha counts: bb.edn's « bash scripts/sandbox/x » runs from the tree, a comment is prose
fn graal_commits(dir: &Path) -> (String, String) {
    let a = commit(
        dir,
        &[
            ("scripts/sandbox/launch.sh", GRAAL_LAUNCH),
            (".devcontainer/flare-sandbox.sb", "(version 1)\n"),
            (
                "scripts/run.sh",
                "git show \"$SHA:scripts/sandbox/launch.sh\"\n",
            ),
        ],
    );
    let s = commit(
        dir,
        &[
            ("scripts/sandbox/new.sh", "new\n"),
            (".devcontainer/flare-sandbox.sb", "(version 1)\n;; more\n"),
            (
                "scripts/run.sh",
                "git show \"$SHA:scripts/sandbox/launch.sh\"\ngit show \"$SHA:scripts/sandbox/new.sh\"\n# $SHA:scripts/sandbox/other.sh\n",
            ),
            (
                "bb.edn",
                r#"{"tasks": {"~#map": [["~$new", {"~#list": ["~$shell", "bash", "scripts/sandbox/new.sh"]}]]}}"#,
            ),
        ],
    );
    (a, s)
}

#[test]
fn graal_a_file_outside_its_approved_set_reading_a_file_the_approved_commit_lacks_fails_it() {
    let _budget = TestBudget::new();
    let r = repo("gate-graal-lacks");
    let (a, s) = graal_commits(&r.main);
    assert_eq!(
        graal_lacks(&r.main, &a, &s).expect("read"),
        ["scripts/run.sh:2: scripts/sandbox/new.sh"]
    );
    assert_eq!(
        graal_lacks(&r.main, &a, &a).expect("read"),
        Vec::<String>::new(),
        "the approved commit judged against itself lacks nothing"
    );
}

#[test]
fn graal_a_worktree_of_a_checkout_named_graal_is_judged_by_graal_s_approved_file() {
    let _budget = TestBudget::new();
    let r = repo("gate-graal");
    let root = r.main.parent().expect("root");
    let main = root.join("graal");
    fs::create_dir_all(&main).expect("graal");
    git(&main, &["init", "-q", "-b", "main"]);
    let (a, s) = graal_commits(&main);
    let wt = root.join("graal-wt");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            wt.to_str().expect("utf-8"),
            &s,
        ],
    );
    let f = root.join("graal-approved");
    fs::write(&f, format!("{a}\n")).expect("approved");
    let out = merge::run(&wt, Some(&f)).expect_err("refused");
    assert!(
        out.contains("scripts/run.sh:2: scripts/sandbox/new.sh\n"),
        "{out}"
    );
    assert!(out.contains("graal's scripts/sandbox/approve.sh"), "{out}");

    fs::write(&f, format!("{s}\n")).expect("approved");
    assert!(
        merge::run(&wt, Some(&f))
            .expect("OK")
            .starts_with("merge gate OK")
    );

    fs::remove_file(&f).expect("rm");
    let why = merge::run(&wt, Some(&f)).expect_err("no record");
    assert!(
        why.starts_with("merge gate: cannot judge: no approved commit on record: "),
        "{why}"
    );
}

/// PR 311's real merge, 44e8cfd9 over master 78cb0f12: a Mac test, as CI's checkout may lack those commits.
#[cfg(target_os = "macos")]
#[test]
fn on_pr_311_s_real_merge() {
    let _budget = TestBudget::new();
    let Some(src) = dash_meter::monorepo() else {
        return;
    };
    let root = TestDir::new("gate-311-real");
    let main = root.0.join("c311");
    let path = |p: &Path| p.to_str().expect("utf-8").to_string();
    git(
        &root.0,
        &[
            "clone",
            "-q",
            "--shared",
            "--no-checkout",
            &path(&src),
            &path(&main),
        ],
    );
    let wt = root.0.join("wt");
    git(
        &main,
        &["worktree", "add", "-q", "--detach", &path(&wt), "44e8cfd9"],
    );
    approve(&main, &git(&main, &["rev-parse", "78cb0f12"]));
    let out = merge::run(&wt, None).expect_err("refused");
    for w in [
        "Makefile:41: sandbox-apply launch",
        "herdr/session.sh:307: sandbox-apply launch",
        "herdr/config.toml:24: sandbox-apply launch",
        "herdr/vibe-task.sh:46: sandbox-apply launch",
        "bin/ci-runner:109: host/sandbox/jobs.conf",
    ] {
        assert!(out.contains(w), "{w} in {out}");
    }
    // MERGED scripts: their host copy waited for his tap
    assert!(
        !out.contains("bin/crm-phone")
            && !out.contains("bin/claude-supervise")
            && !out.contains("handoff.ex"),
        "{out}"
    );
    approve(&main, &git(&main, &["rev-parse", "44e8cfd9"]));
    assert!(
        merge::run(&wt, None)
            .expect("OK")
            .starts_with("merge gate OK: ")
    );
}

/// Done-when 2 of rust-merge-gate-port: each case of the retired Elixir suite, by its own name, and the Rust test that keeps it.
const PORTED: [(&str, &str); 14] = [
    (
        "PR 311's merge fails before his tap, naming each file and line and what the copy lacks",
        "pr_311_s_merge_fails_before_his_tap_naming_each_file_and_line",
    ),
    (
        "a MERGED script and the Makefile's approved half call a new verb ahead of his tap; its other half may not",
        "a_merged_script_and_the_makefile_s_approved_half_call_a_new_verb_ahead_of_his_tap",
    ),
    (
        "a MERGED call to a verb the PR's own script lacks is a hit",
        "a_merged_call_to_a_verb_the_pr_s_own_script_lacks_is_a_hit",
    ),
    (
        "approved_half/1 follows nested and else branches",
        "approved_half_follows_nested_and_else_branches",
    ),
    (
        "approved_half/1 reads a recipe's shell if and else as recipe text",
        "approved_half_reads_a_recipe_s_shell_if_and_else_as_recipe_text",
    ),
    (
        "it passes once 311 is the approved copy",
        "it_passes_once_311_is_the_approved_copy",
    ),
    (
        "the main checkout is not judged; no approval on record, or no checkout, is never OK",
        "the_main_checkout_is_not_judged_no_approval_or_no_checkout_is_never_ok",
    ),
    (
        "a verb in a label of several, `  launch | run)`, and a priv/commands caller are seen",
        "a_verb_in_a_label_of_several_and_a_priv_commands_caller_are_seen",
    ),
    (
        "an approved commit it cannot read fails it, never OK",
        "an_approved_commit_it_cannot_read_fails_it_never_ok",
    ),
    (
        "an approved script whose SET or MERGED it cannot read fails it, never OK",
        "an_approved_script_whose_set_it_cannot_read_fails_it_never_ok",
    ),
    (
        "graal: a file outside its approved set reading a scripts/sandbox file the approved commit lacks fails it",
        "graal_a_file_outside_its_approved_set_reading_a_file_the_approved_commit_lacks_fails_it",
    ),
    (
        "graal: a worktree of a checkout named graal is judged by graal's approved file and rules",
        "graal_a_worktree_of_a_checkout_named_graal_is_judged_by_graal_s_approved_file",
    ),
    (
        "dash sandbox gate answers it, and names its usage without a folder",
        "dash_sandbox_gate_answers_and_names_its_usage_without_a_folder",
    ),
    (
        "on PR 311's real merge, 44e8cfd9 over master 78cb0f12",
        "on_pr_311_s_real_merge",
    ),
];

/// Every `test "…"` of private/dashboard/test/dashboard/merge_gate_test.exs has its Rust test here or in dash's
/// tests/sandbox_gate.rs (the CLI case), and none is left out: a case added there and not here fails.
#[test]
fn every_elixir_merge_gate_case_has_its_rust_test() {
    let _budget = TestBudget::new();
    let Some(top) = dash_meter::monorepo() else {
        return;
    };
    let read = |p: &str| fs::read_to_string(top.join(p)).expect(p);
    let elixir = read("private/dashboard/test/dashboard/merge_gate_test.exs");
    let rust = read("rust/crates/gate/tests/merge_gate.rs")
        + &read("rust/crates/dash/tests/sandbox_gate.rs");
    let cases: Vec<&str> = elixir
        .lines()
        .filter_map(|l| l.trim_start().strip_prefix("test \"")?.split('"').next())
        .collect();
    let named: Vec<&str> = PORTED.iter().map(|(e, _)| *e).collect();
    assert_eq!(
        cases, named,
        "give each Elixir case above a line in PORTED, its Rust test beside it"
    );
    for (case, test) in PORTED {
        println!("{case}  =>  {test}");
        assert!(
            rust.contains(&format!("fn {test}()")),
            "write the Rust test {test} for « {case} »"
        );
    }
}
