//! What a folder under herdr's worktrees is, read from its path: Rust's port of the older app's readers that name a task from its
//! folder, `Launch.task_of_folder`, `Launch.rust_folder?`, Wake's task check and `Watch.Panes.repo` (task rust-worktrees-noindex).
//! A path is read as given (`~/.herdr/worktrees/<repo>/<name>`) or real (`~/.herdr/worktrees.noindex/.herdr/worktrees/...`, what
//! getcwd, lsof and herdr's panes say once the root is a link): both end in the tail `/.herdr/worktrees/<repo>/<name>`.
use std::path::Path;

const TAIL: &str = "/.herdr/worktrees/";

/// The repository, the worktree's folder name and whether `cwd` is that folder itself, from the first `TAIL` in it.
fn under(cwd: &str) -> Option<(&str, &str, bool)> {
    let rest = &cwd[cwd.find(TAIL)? + TAIL.len()..];
    let mut parts = rest.splitn(3, '/');
    let (repo, name) = (parts.next()?, parts.next()?);
    // Wake's `task-[^/]+$`: the folder itself, no trailing slash
    let root = parts.next().is_none();
    (!repo.is_empty() && !name.is_empty()).then_some((repo, name, root))
}

/// `Launch.task_of_folder`: the project and task of a herdr delegate's worktree (`task-<id>`) holding `cwd`, at its top or below.
/// A folder under `repos` (`~/repo`) is never a task's, though a stand-in's tree there may hold such a folder.
pub fn task_of_folder<'a>(cwd: &'a str, repos: &Path) -> Option<(&'a str, &'a str)> {
    if Path::new(cwd).starts_with(repos) {
        return None;
    }
    let (repo, name, _) = under(cwd)?;
    let id = name.strip_prefix("task-").filter(|id| !id.is_empty())?;
    Some((repo, id))
}

/// The task whose worktree serves TCP `port` on this Mac: the folder of what listens there, read with lsof, as the older app's
/// `Comments.task_of_url` read it (task capture-extension: a sending from a task's own page names that task).
pub fn task_serving(port: u16) -> Option<String> {
    let limit = std::time::Duration::from_secs(5);
    let tcp = format!("-iTCP:{port}");
    let pids =
        dash_process::run("lsof", &["-wnP", &tcp, "-sTCP:LISTEN", "-t"], None, limit).ok()?;
    let pids: Vec<&str> = pids.split_whitespace().collect();
    if pids.is_empty() {
        return None;
    }
    let at = ["-w", "-a", "-p", &pids.join(","), "-d", "cwd", "-Fn"];
    let cwds = dash_process::run("lsof", &at, None, limit).ok()?;
    let repos = Path::new(&std::env::var("HOME").ok()?).join("repo");
    (cwds.lines().filter_map(|l| l.strip_prefix('n')))
        .find_map(|cwd| task_of_folder(cwd, &repos).map(|(_, id)| id.to_string()))
}

/// `dash hook linkedin-budget`'s task: the task of the session whose folder is `cwd` and its brief, the only LinkedIn calls it
/// may make. A delegate's (`task-<id>`) is the file herdr/session.sh wrote, `<home>/.claude/briefs/<repo>-<id>.md`, "" when it
/// wrote none; a Rust run's (`rust-<id>`) is typed in its pane, never a file, so `filed` writes it again from the list, "" when
/// it cannot. None only for a session of no task (the main checkout, any folder under `<home>/repo`), which the budget alone
/// holds: a task read as none would call LinkedIn past its brief, so every shape of a task's folder names it, given or real,
/// at its top or below, a half-made `.new` too (no such task, so "": no call). Send-path review of PR 1574: a Rust run's
/// worktree named none, so its task called LinkedIn as the main session does (@nha, 2026-10-20: « we should never hit a
/// rate limit. This is putting us in danger to get linked in bannned »).
pub fn task_brief(
    cwd: &str,
    home: &Path,
    filed: impl Fn(&str) -> String,
) -> Option<(String, String)> {
    let repos = home.join("repo");
    if let Some((repo, id)) = task_of_folder(cwd, &repos) {
        let brief = std::fs::read_to_string(home.join(format!(".claude/briefs/{repo}-{id}.md")));
        return Some((id.to_string(), brief.unwrap_or_default()));
    }
    if Path::new(cwd).starts_with(&repos) {
        return None;
    }
    let id = under(cwd)?.1.strip_prefix("rust-")?;
    (!id.is_empty()).then(|| (id.to_string(), filed(id)))
}

/// `Launch.rust_folder?`: whether `cwd` sits in a worktree of Rust's orchestrator (`rust-[^/]+`, a half-made `.new` too), whose
/// run owns its panes.
pub fn rust_folder(cwd: &str) -> bool {
    under(cwd).is_some_and(|(_, name, _)| name.len() > "rust-".len() && name.starts_with("rust-"))
}

/// Wake's task check: whether an agent's folder is a delegate's worktree itself, a task session, never one below it.
pub fn task_root(cwd: &str, repos: &Path) -> bool {
    under(cwd).is_some_and(|(_, _, root)| root) && task_of_folder(cwd, repos).is_some()
}

/// `Watch.Panes.repo`: the repository a pane's folder belongs to: a herdr worktree's, else the folder under `repos` (`~/repo`),
/// else the folder's own name. A folder under `repos` is its checkout's first: Elixir's worktrees root is never below the repos
/// root, so the deeper one is the repos root whenever both hold it, a stand-in's repos root in a worktree or a stand-in's tree
/// in a checkout (/code-review of PR 1574), as `task_of_folder` reads it.
pub fn repo<'a>(cwd: &'a str, repos: &Path) -> &'a str {
    let below = Path::new(cwd).strip_prefix(repos).ok();
    let checkout = below.and_then(|b| b.iter().next()).and_then(|n| n.to_str());
    match (checkout, under(cwd)) {
        (Some(checkout), _) => checkout,
        (None, Some((repo, _, _))) => repo,
        (None, None) => cwd.rsplit('/').find(|s| !s.is_empty()).unwrap_or(cwd),
    }
}

/// `cwd` below `root`, `root` matched as given or as its real path: herdr and getcwd read a folder real.
pub fn below<'a>(cwd: &'a str, root: &Path) -> Option<&'a Path> {
    let under = |r: &Path| Path::new(cwd).strip_prefix(r).ok();
    under(root).or_else(|| under(&std::fs::canonicalize(root).ok()?))
}

/// The repository and task whose PR a reviewer's session in `cwd` works on: `…/<repo>/task-review-<id>`, the older app's
/// reviewer worktree, at its top or below.
pub fn reviewer_of(cwd: &str) -> Option<(&str, &str)> {
    let (repo, name, _) = under(cwd)?;
    Some((repo, name.strip_prefix("task-review-")?))
}

/// The PR's own worktree, `<the lane's checkout>/.claude/run/pr-lane/site_perso-<n>`, for `dash review next` from `cwd`: one
/// checkout per PR, so a push never carries another PR's merge (the older app's `PrLane.worktree`, task review-next-held). One
/// there already is kept.
pub fn pr_worktree(n: u64, cwd: &Path) -> Result<std::path::PathBuf, String> {
    let git = |args: &[&str]| {
        let said = dash_process::answer(
            "git",
            args,
            Some(cwd),
            &[],
            std::time::Duration::from_secs(30),
        );
        match said.map_err(|e| format!("git {}: {e:?}", args.join(" ")))? {
            (status, out, _) if status.success() => Ok(out),
            (_, _, err) => Err(format!("git {}: {}", args.join(" "), err.trim())),
        }
    };
    let top = git(&["rev-parse", "--show-toplevel"])?;
    // run from inside a PR's worktree, the folder still hangs off the lane's checkout, never nested
    let lane = top
        .trim()
        .split("/.claude/run/pr-lane/")
        .next()
        .unwrap_or_default();
    let wt = Path::new(lane).join(format!(".claude/run/pr-lane/site_perso-{n}"));
    let at = wt.to_string_lossy();
    // a folder removed by hand leaves git's registration, which refuses the add
    let _ = git(&["worktree", "prune"]);
    if listed(&git(&["worktree", "list", "--porcelain"])?, &wt) {
        return Ok(wt);
    }
    if wt.exists() {
        return Err(format!(
            "{at} is no registered worktree: remove it, then run review next again"
        ));
    }
    // no hooks: the checkout's post-checkout hook is the session's, not this folder's
    let add = [
        "-c",
        "core.hooksPath=/dev/null",
        "worktree",
        "add",
        "--detach",
        &at,
        "origin/main",
    ];
    git(&add).map(|_| wt)
}

/// Whether `git worktree list --porcelain` lists `wt`, matched real: one added before ~/.herdr/worktrees became a link is listed
/// at its old path, read real since (/code-review of PR 1574: `dash review next` refused a PR's own worktree as unregistered).
pub fn listed(porcelain: &str, wt: &Path) -> bool {
    let mut paths = porcelain
        .lines()
        .filter_map(|l| l.strip_prefix("worktree "));
    std::fs::canonicalize(wt)
        .is_ok_and(|real| paths.any(|p| std::fs::canonicalize(p).is_ok_and(|p| p == real)))
}

/// `dash worktree of <folder>`: the four readers' answers on a folder, one line each.
pub fn describe(folder: &str, repos: &Path) -> String {
    let task = task_of_folder(folder, repos).map_or("none".into(), |(p, id)| format!("{p} {id}"));
    format!(
        "repo {}\ntask {task}\nrust {}\ntask root {}\n",
        repo(folder, repos),
        rust_folder(folder),
        task_root(folder, repos)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// A reviewer's session is read from its folder: `task-review-<id>`, at its top or below.
    #[dash_test]
    fn a_reviewer_is_read_from_its_folder() {
        let of = reviewer_of;
        assert_eq!(
            of("/u/.herdr/worktrees/site_perso/task-review-rust-x/rust"),
            Some(("site_perso", "rust-x"))
        );
        assert_eq!(
            of("/u/.herdr/worktrees/klar/task-review-x"),
            Some(("klar", "x"))
        );
        assert_eq!(of("/u/.herdr/worktrees/site_perso/task-rust-y"), None);
        assert_eq!(of("/u/repo/site_perso"), None);
    }

    /// /code-review of PR 1574: a PR's worktree added before its root became a link is listed at the link's path, while the
    /// lane reads its checkout real; it is still found, never refused as unregistered.
    #[dash_test]
    fn a_pr_worktree_added_before_the_move_is_found_at_the_real_path() {
        let dir = dash_meter::TestDir::new("review-lane-moved");
        let (old, real) = (dir.0.join("worktrees"), dir.0.join("real"));
        let repo = old.join("lane");
        std::fs::create_dir_all(&repo).expect("dir");
        for args in [
            &["init", "-q"][..],
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
            &[
                "worktree",
                "add",
                "-q",
                "--detach",
                ".claude/run/pr-lane/site_perso-7",
                "HEAD",
            ],
        ] {
            let quiet = [&["-c", "core.hooksPath=/dev/null"][..], args].concat();
            let ran = std::process::Command::new("git")
                .args(&quiet)
                .current_dir(&repo)
                .status();
            assert!(ran.expect("git").success());
        }
        std::fs::rename(&old, &real).expect("moved");
        std::os::unix::fs::symlink(&real, &old).expect("linked");
        let lane = std::fs::canonicalize(real.join("lane")).expect("real");
        let wt = pr_worktree(7, &lane).expect("found");
        assert!(wt.ends_with(".claude/run/pr-lane/site_perso-7"), "{wt:?}");
    }

    const GIVEN: &str = "/Users/n/.herdr/worktrees/site_perso";
    const REAL: &str = "/Users/n/.herdr/worktrees.noindex/.herdr/worktrees/site_perso";

    fn repos() -> &'static Path {
        Path::new("/Users/n/repo")
    }

    /// Task capture-extension: what listens on a port names the task of the folder it runs in, here this test's own (a task's
    /// worktree on his Mac, none elsewhere); a port nothing listens on names none.
    #[dash_test]
    fn the_task_serving_a_port_is_its_listener_s_folder_s() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
        let port = listener.local_addr().expect("its address").port();
        let cwd = std::env::current_dir()
            .expect("cwd")
            .to_string_lossy()
            .into_owned();
        let repos = Path::new(&std::env::var("HOME").expect("HOME")).join("repo");
        let own = task_of_folder(&cwd, &repos).map(|(_, id)| id.to_string());
        assert_eq!(task_serving(port), own, "{cwd}");
        drop(listener);
        assert_eq!(task_serving(port), None);
    }

    /// `Launch.task_of_folder`, ported: a delegate's worktree names its project and task at its top and below, either path; a
    /// stand-in's tree under the repos root names none.
    #[dash_test]
    fn task_of_folder_names_a_delegate_s_task_either_path() {
        let of =
            |cwd: &str| task_of_folder(cwd, repos()).map(|(r, id)| (r.to_string(), id.to_string()));
        let named = Some(("site_perso".to_string(), "cg8".to_string()));
        for root in [GIVEN, REAL] {
            assert_eq!(of(&format!("{root}/task-cg8")), named);
            assert_eq!(of(&format!("{root}/task-cg8/private")), named);
            assert_eq!(of(&format!("{root}/rust-cg8")), None, "Rust's run");
            assert_eq!(of(&format!("{root}/task-")), None);
        }
        assert_eq!(
            of("/Users/n/repo/site_perso/tmp/.herdr/worktrees/klar/task-x"),
            None,
            "a stand-in in a checkout"
        );
    }

    /// The send path's reader, each shape of a session's folder (send-path review of PR 1574): a delegate's worktree and a Rust
    /// run's name their task and its brief, at the top or below, through the link or at the real path under .noindex, on a
    /// real tree; a task with no brief has "" (so it calls nothing); the main checkout, a stand-in under the repos root, a
    /// pool's slot and a folder elsewhere name none.
    #[dash_test]
    fn task_brief_names_the_session_s_task_for_every_shape_of_its_folder() {
        let dir = dash_meter::TestDir::new("folder-task-brief");
        let home = std::fs::canonicalize(&dir.0).expect("real");
        let real = home.join(".herdr/worktrees.noindex/.herdr/worktrees/site_perso");
        for folder in [
            "task-li-check/private",
            "rust-rust-a/rust/crates",
            "pool.noindex/pool-1",
        ] {
            std::fs::create_dir_all(real.join(folder)).expect("a worktree");
        }
        std::fs::create_dir_all(home.join("repo/site_perso/tmp")).expect("the main checkout");
        std::fs::create_dir_all(home.join(".claude/briefs")).expect("briefs");
        std::fs::write(
            home.join(".claude/briefs/site_perso-li-check.md"),
            "get_inbox",
        )
        .expect("a brief");
        let link = home.join(".herdr/worktrees");
        std::os::unix::fs::symlink("worktrees.noindex/.herdr/worktrees", &link).expect("a link");
        let filed = |id: &str| match id {
            "rust-a" => "search_jobs".to_string(),
            _ => String::new(),
        };
        let of = |cwd: &Path| task_brief(&cwd.to_string_lossy(), &home, filed);
        let named = |id: &str, brief: &str| Some((id.to_string(), brief.to_string()));
        let given = link.join("site_perso");
        assert!(given.join("task-li-check").is_dir(), "the link holds them");
        for root in [&given, &real] {
            for below in ["", "private"] {
                let at = root.join("task-li-check").join(below);
                assert_eq!(of(&at), named("li-check", "get_inbox"), "{at:?}");
                let real_at = std::fs::canonicalize(&at).expect("real");
                assert!(real_at.starts_with(&real), "{real_at:?} is under .noindex");
                assert_eq!(of(&real_at), named("li-check", "get_inbox"));
            }
            for below in ["", "rust", "rust/crates"] {
                let at = root.join("rust-rust-a").join(below);
                assert_eq!(of(&at), named("rust-a", "search_jobs"), "{at:?}");
            }
            assert_eq!(of(&root.join("task-other")), named("other", ""));
            assert_eq!(of(&root.join("rust-gone")), named("gone", ""));
            assert_eq!(
                of(&root.join("rust-rust-a.new")),
                named("rust-a.new", ""),
                "half-made: no such task, no call"
            );
            assert_eq!(of(&root.join("pool.noindex/pool-1")), None, "a slot");
            assert_eq!(of(&root.join("rust-")), None);
            assert_eq!(of(root), None);
        }
        let checkout = home.join("repo/site_perso");
        assert_eq!(of(&checkout), None, "the main checkout");
        assert_eq!(of(&checkout.join("tmp")), None, "below it");
        for stand_in in ["task-li-check", "rust-rust-a"] {
            let at = (checkout.join("tmp/.herdr/worktrees/site_perso")).join(stand_in);
            assert_eq!(of(&at), None, "a stand-in under the repos root");
        }
        assert_eq!(of(&home.join("Desktop")), None);
    }

    /// `Launch.rust_folder?`, ported: a Rust run's worktree, a half-made `.new` too, never a delegate's or a slot, either path.
    #[dash_test]
    fn rust_folder_is_a_rust_run_s_worktree_either_path() {
        for root in [GIVEN, REAL] {
            assert!(rust_folder(&format!("{root}/rust-rust-a")));
            assert!(rust_folder(&format!("{root}/rust-rust-a/rust/crates")));
            assert!(
                rust_folder(&format!("{root}/rust-rust-a.new")),
                "half-made, the run's"
            );
            assert!(!rust_folder(&format!("{root}/task-a")));
            assert!(!rust_folder(&format!("{root}/pool.noindex/pool-1")));
        }
        assert!(!rust_folder("/Users/n/repo/site_perso"));
    }

    /// Wake's task check, ported: an agent in a delegate's worktree itself is a task session; below it, with a trailing slash
    /// (Wake's `$`) or elsewhere, not.
    #[dash_test]
    fn task_root_is_a_delegate_s_worktree_itself_either_path() {
        for root in [GIVEN, REAL] {
            assert!(task_root(&format!("{root}/task-x"), repos()));
            assert!(!task_root(&format!("{root}/task-x/"), repos()));
            assert!(!task_root(&format!("{root}/task-x/private"), repos()));
            assert!(!task_root(&format!("{root}/rust-x"), repos()));
        }
        assert!(!task_root("/Users/n/repo/site_perso", repos()));
    }

    /// `Watch.Panes.repo`, ported: a herdr worktree's repository either path, a checkout's under the repos root, the deeper
    /// root first, else the folder's name.
    #[dash_test]
    fn repo_of_a_pane_s_folder_either_path() {
        for root in [GIVEN, REAL] {
            assert_eq!(
                repo(&format!("{root}/task-x/private"), repos()),
                "site_perso"
            );
            assert_eq!(repo(&format!("{root}/rust-y"), repos()), "site_perso");
        }
        assert_eq!(
            repo("/home/runner/.herdr/worktrees/klar/task-z", repos()),
            "klar",
            "any home"
        );
        assert_eq!(repo("/Users/n/repo/klar/model-lab", repos()), "klar");
        assert_eq!(repo("/Users/n/Desktop/notes", repos()), "notes");
        let inside = Path::new("/Users/n/.herdr/worktrees/site_perso/task-a/tmp/repo");
        assert_eq!(
            repo(&format!("{}/klar", inside.display()), inside),
            "klar",
            "a stand-in's repos root in a worktree"
        );
        assert_eq!(
            repo(
                "/Users/n/repo/site_perso/tmp/.herdr/worktrees/klar/task-x",
                repos()
            ),
            "site_perso",
            "a stand-in's tree in a checkout"
        );
    }
}
