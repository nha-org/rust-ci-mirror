//! `dash worktree lease|leave|list --repo`, the real binary, on a repository that is not site_perso (task reap-kept-worktrees).
use dash_meter::TestDir;
use dash_meter::dash_test;
use std::path::Path;
use std::process::{Command, Output};

fn git(at: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(at)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn dash(home: &Path, args: &[&str]) -> (i32, String) {
    let Output {
        status,
        stdout,
        stderr,
    } = Command::new(env!("CARGO_BIN_EXE_dash"))
        .args(["worktree"])
        .args(args)
        .env("HOME", home)
        .output()
        .expect("dash worktree");
    let said = String::from_utf8_lossy(&stdout).into_owned() + &String::from_utf8_lossy(&stderr);
    (status.code().expect("an exit"), said)
}

/// Done-when 3: klar's checkout (a repository named klar, cloned) leaves a worktree to its own pool under
/// ~/.herdr/worktrees/klar, lists it, and leases it back on a new branch; a second lease finds none free (exit 3).
#[dash_test]
fn another_repository_leases_leaves_and_lists_its_own_pool() {
    let dir = TestDir::new("dash-worktree-klar");
    let real = std::fs::canonicalize(&dir.0).expect("real");
    let (origin, klar, home) = (
        real.join("origin"),
        real.join("repo/klar"),
        real.join("home"),
    );
    std::fs::create_dir_all(&origin).expect("dirs");
    std::fs::create_dir_all(&home).expect("dirs");
    git(&origin, &["init", "-q", "-b", "main"]);
    git(
        &origin,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "first",
        ],
    );
    let at = |p: &Path| p.to_string_lossy().into_owned();
    git(&real, &["clone", "-q", &at(&origin), &at(&klar)]);
    let wt = real.join("wt/task-a");
    git(
        &klar,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "task/a",
            &at(&wt),
            "origin/main",
        ],
    );
    let repo = at(&klar);

    let (code, said) = dash(&home, &["leave", "--repo", &repo, &at(&wt)]);
    assert_eq!((code, said), (0, format!("left: {}\n", at(&wt))));
    let slot = home.join(".herdr/worktrees/klar/pool.noindex/pool-1");
    assert!(
        slot.join(".git").exists() && !wt.exists(),
        "klar's own pool"
    );

    let (code, said) = dash(&home, &["list", "--repo", &repo]);
    assert_eq!(code, 0, "{said}");
    assert_eq!(
        said,
        format!("pool\t0h\t(detached)\t{}\n0 unheld past a day\n", at(&slot))
    );

    let task = home.join(".herdr/worktrees/klar/task-b");
    let lease = [
        "lease",
        "--repo",
        &repo,
        "--path",
        &at(&task),
        "--branch",
        "task/b",
        "--base",
        "origin/main",
    ];
    let (code, said) = dash(&home, &lease);
    assert_eq!(
        (code, said),
        (
            0,
            format!("leased: {} on task/b, from the pool\n", at(&task))
        )
    );
    assert_eq!(git(&task, &["branch", "--show-current"]).trim(), "task/b");
    assert!(!slot.exists(), "the slot is its worktree now");
    let (code, said) = dash(&home, &["list", "--repo", &repo]);
    assert_eq!(
        said,
        format!("held\t0h\ttask/b\t{}\n0 unheld past a day\n", at(&task)),
        "{code}"
    );

    let again = [
        "lease",
        "--repo",
        &repo,
        "--path",
        &at(&real.join("wt/c")),
        "--branch",
        "task/c",
        "--base",
        "origin/main",
    ];
    assert_eq!(
        dash(&home, &again),
        (3, "no slot free: make a fresh worktree\n".into())
    );
    assert_eq!(dash(&home, &["list"]).0, 2, "a misuse");
}

/// A stand-in program at `path`: a bash script.
fn script(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().expect("a folder")).expect("dirs");
    std::fs::write(path, format!("#!/usr/bin/env bash\n{body}")).expect("write");
    let exec = std::os::unix::fs::PermissionsExt::from_mode(0o755);
    std::fs::set_permissions(path, exec).expect("chmod");
}

/// herdr/session.sh on a stand-in herdr and dash (delegate-test's set-up, which the Elixir gate keeps unchanged), from the pane
/// `pane` when one is named, as `end` runs. `None` on the public mirror, which holds rust/ alone.
fn session(t: &Path, free: bool, args: &[&str], pane: Option<&str>) -> Option<Output> {
    let (wt, bin, home) = (t.join("wt"), t.join("bin"), t.join("home"));
    let repo = dash_meter::monorepo()?;
    for f in ["herdr/session.sh", "bin/worktree-seed"] {
        std::fs::create_dir_all(wt.join(f).parent().expect("a folder")).expect("dirs");
        std::fs::copy(repo.join(f), wt.join(f)).expect("the checkout's script");
    }
    let lease = format!(
        "[ \"$1\" = worktree ] || exit 0; echo \"$*\" >> {0}/leased; [ \"$2\" != lease ] || [ -e {0}/slot-free ] || exit 3\n",
        t.display()
    );
    script(&wt.join("private/dashboard/bin/dash"), &lease);
    let herdr = format!(
        r#"pane='{{"result":{{"root_pane":{{"pane_id":"w1:p2"}}}}}}'
case "$1 $2" in
  "pane list") echo '{{"result":{{"panes":[{{"pane_id":"w1:p1","workspace_id":"w1","cwd":"{0}/home/repo/klar"}},{{"pane_id":"w1:p2","workspace_id":"w1","cwd":"{0}/x","agent_status":"working"}}]}}}}' ;;
  "agent list") echo '{{"result":{{"agents":[]}}}}' ;;
  "workspace list") cat {0}/workspaces 2>/dev/null || echo '{{"result":{{"workspaces":[]}}}}' ;;
  "workspace close") echo "$*" >> {0}/closed ;;
  "worktree open") echo "$*" >> {0}/opened; echo "$pane" ;;
  "worktree create") echo "$*" >> {0}/created; echo "$pane" ;;
esac
"#,
        t.display()
    );
    script(&bin.join("herdr"), &herdr);
    script(
        &bin.join("sysctl"),
        "echo 'total = 6144.00M  used = 0.00M  free = 1024.00M  (encrypted)'\n",
    );
    script(
        &bin.join("memory_pressure"),
        "echo 'System-wide memory free percentage: 80%'\n",
    );
    if free {
        std::fs::write(t.join("slot-free"), "").expect("a free slot");
    } else {
        let _ = std::fs::remove_file(t.join("slot-free"));
    }
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").expect("PATH"));
    let mut run = Command::new("bash");
    run.arg(wt.join("herdr/session.sh"))
        .args(args)
        .current_dir(t)
        .env("HOME", &home)
        .env("PATH", path)
        .env("SUBMIT_PAUSE", "0")
        .env("DASH_MEMORY_GATE", "app")
        .env("DASH_NO_PRS", "1")
        .env_remove("HERDR_PANE_ID")
        .env_remove("HERDR_ACTIVE_PANE_ID");
    if let Some(pane) = pane {
        run.env("HERDR_PANE_ID", pane)
            .env("SESSION_END_DELAY", "0")
            .env("TASK_PROCS_RUN", t.join("procs"));
    }
    Some(run.output().expect("session.sh"))
}

/// `session.sh delegate klar <extra> --worktree`: what dash was asked to lease, what herdr opened, what it created.
fn delegate(t: &Path, free: bool, extra: &[&str]) -> Option<[String; 3]> {
    for f in ["leased", "opened", "created"] {
        let _ = std::fs::remove_file(t.join(f));
    }
    let args = [&["delegate", "klar"][..], extra, &["--worktree"]].concat();
    let out = session(t, free, &args, None)?;
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "session.sh delegate {extra:?}: {said}"
    );
    let read = |f: &str| std::fs::read_to_string(t.join(f)).unwrap_or_default();
    Some([read("leased"), read("opened"), read("created")])
}

/// Done-when 1, a review start: Dashboard.Orch.PM's launch (`--worktree --task-id review-<id> --model opus --no-mcp`) leases a
/// pool slot through dash and opens it, never creating one.
#[dash_test]
fn a_review_start_leases_a_pool_slot_through_dash() {
    let dir = TestDir::new("dash-worktree-review");
    let t = std::fs::canonicalize(&dir.0).expect("real");
    klar(&t);
    let args = [
        "review PR 9",
        "--task-id",
        "review-x",
        "--model",
        "opus",
        "--no-mcp",
    ];
    let Some([leased, opened, created]) = delegate(&t, true, &args) else {
        return;
    };
    let p = t.join("home/.herdr/worktrees/klar/task-review-x");
    let repo = t.join("home/repo/klar");
    let lease = format!(
        "worktree lease --repo {} --path {} --branch task/review-x --base main\n",
        repo.display(),
        p.display()
    );
    assert_eq!(leased, lease);
    assert!(
        opened.contains(&format!("--path {} ", p.display())),
        "{opened}"
    );
    assert_eq!(created, "", "a leased start creates none");
}

/// Done-when 1, a delegate: `session.sh delegate --worktree` leases a pool slot through dash and opens it; with none free
/// (exit 3) herdr creates one, as before the pool.
#[dash_test]
fn a_delegate_leases_a_pool_slot_through_dash_and_creates_only_when_none_is_free() {
    let dir = TestDir::new("dash-worktree-delegate");
    let t = std::fs::canonicalize(&dir.0).expect("real");
    klar(&t);
    let Some([leased, opened, created]) = delegate(&t, true, &["pooled", "--task-id", "d1"]) else {
        return;
    };
    let p = t.join("home/.herdr/worktrees/klar/task-d1");
    assert!(leased.contains("--branch task/d1 --base main"), "{leased}");
    assert!(
        opened.contains(&format!("--path {} ", p.display())),
        "{opened}"
    );
    assert_eq!(created, "", "a leased start creates none");
    let Some([leased, opened, created]) = delegate(&t, false, &["fresh", "--task-id", "d2"]) else {
        return;
    };
    let p = t.join("home/.herdr/worktrees/klar/task-d2");
    assert!(leased.contains("--branch task/d2"), "asked first: {leased}");
    assert_eq!(opened, "", "nothing leased, nothing opened");
    assert!(
        created.contains(&format!("--path {} ", p.display())),
        "{created}"
    );
}

/// A klar checkout with no commit, as delegate-test's: session.sh reads its default branch, `main`.
fn klar(t: &Path) {
    let repo = t.join("home/repo/klar");
    std::fs::create_dir_all(&repo).expect("dirs");
    git(&repo, &["init", "-q", "-b", "main"]);
}

/// Review finding 1: `session.sh end --merged` in a delegate's sub-row closes its workspace, then gives the worktree back
/// through `dash worktree leave`, never deleting it with its build as herdr's `worktree remove` did.
#[dash_test]
fn a_delegate_s_end_leaves_its_worktree_to_the_pool() {
    let dir = TestDir::new("dash-worktree-end");
    let t = std::fs::canonicalize(&dir.0).expect("real");
    klar(&t);
    let repo = t.join("home/repo/klar");
    git(
        &repo,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "first",
        ],
    );
    let wt = t.join("home/.herdr/worktrees/klar/task-e1");
    let at = wt.to_string_lossy().into_owned();
    git(&repo, &["worktree", "add", "-q", "-b", "task/e1", &at]);
    let listed = format!(
        r#"{{"result":{{"workspaces":[{{"workspace_id":"w1","worktree":{{"is_linked_worktree":true,"checkout_path":"{at}"}}}}]}}}}"#
    );
    std::fs::write(t.join("workspaces"), listed).expect("its workspace");
    let Some(out) = session(&t, false, &["end", "--merged"], Some("w1:p2")) else {
        return;
    };
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // its job runs in the background: up to 30 s for the leave
    let read = |f: &str| std::fs::read_to_string(t.join(f)).unwrap_or_default();
    let began = std::time::Instant::now();
    while read("leased").is_empty() && began.elapsed() < std::time::Duration::from_secs(30) {
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    assert_eq!(
        read("closed"),
        "workspace close w1\n",
        "its sub-row closed first"
    );
    assert_eq!(read("leased"), format!("worktree leave --repo {at} {at}\n"));
}

/// Task rust-worktrees-noindex, done-when 2: the older app's readers, ported, answer one command on a folder read real, as
/// herdr's panes and getcwd give it once ~/.herdr/worktrees is a link.
#[dash_test]
fn worktree_of_names_a_real_path_s_repo_and_task() {
    let dir = TestDir::new("dash-worktree-of");
    let real = "/Users/n/.herdr/worktrees.noindex/.herdr/worktrees/site_perso/task-cg8";
    assert_eq!(
        dash(&dir.0, &["of", real]),
        (
            0,
            "repo site_perso\ntask site_perso cg8\nrust false\ntask root true\n".into()
        )
    );
    let below = "/Users/n/.herdr/worktrees.noindex/.herdr/worktrees/site_perso/rust-rust-a/rust";
    assert_eq!(
        dash(&dir.0, &["of", below]),
        (
            0,
            "repo site_perso\ntask none\nrust true\ntask root false\n".into()
        )
    );
}
