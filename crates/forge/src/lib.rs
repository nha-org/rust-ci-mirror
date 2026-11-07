//! Core's Forge on git and GitHub's CLI. A task's worktree is `rust-<id>` beside the others, its branch `task/<id>`, which CI's
//! title check, the task page and the older app's PR reads all expect.
use dash_core::tasks::{Checks, Forge, Pr};
use dash_process as process;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub struct GitHub {
    /// The checkout its worktrees hang off.
    pub repo: PathBuf,
    /// The folder they sit in.
    pub root: PathBuf,
    pub base: String,
}

const CALL: Duration = Duration::from_secs(30);
/// A fetch and a worktree of a large repository.
const GIT: Duration = Duration::from_secs(300);

/// What dash ci wait and the merge hook count as passed (merge-claim.py GREEN).
const GREEN: [&str; 3] = ["SUCCESS", "NEUTRAL", "SKIPPED"];

/// The real path: Claude names a transcript's folder after it, so /tmp/x or a relative path would never match. Refused under
/// a `task-<id>` folder, where the older app's Launch.task_of_folder would adopt the pane (point p-adopt).
fn real_folder(folder: &Path) -> Result<String, String> {
    let real = std::fs::canonicalize(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let named = |c: std::path::Component| c.as_os_str().to_string_lossy().starts_with("task-");
    if real.components().any(named) {
        return Err(format!(
            "{}: a task-<id> folder is the older app's to adopt; give a folder named otherwise",
            real.display()
        ));
    }
    Ok(real.to_string_lossy().into_owned())
}

/// The PR holding this branch's work, in `gh pr list`'s answer of its newest. A closed one holds nothing, and an open or merged
/// one only what its head holds: a commit not pushed, or a branch used again after one landed, is work no PR holds yet.
fn pr_in(text: &str, local_head: Option<&str>) -> Result<Option<Pr>, String> {
    let list: Value =
        serde_json::from_str(text).map_err(|e| format!("gh's answer: {e}: {text}"))?;
    let Some(pr) = list.as_array().and_then(|l| l.first()) else {
        return Ok(None);
    };
    let word = |field: &str| pr[field].as_str().unwrap_or_default().to_string();
    let number = pr["number"]
        .as_u64()
        .ok_or(format!("no number in: {text}"))?;
    let (head, merged) = (word("headRefOid"), pr["state"] == "MERGED");
    if pr["state"] == "CLOSED" || local_head.is_some_and(|l| l != head) {
        return Ok(None);
    }
    Ok(Some(Pr {
        number,
        url: word("url"),
        head,
        merged,
    }))
}

/// `merge-claim.py --stale`'s answer: the PR's files its base branch changed since the head last met it.
fn stale_in(text: &str) -> Result<Vec<String>, String> {
    let answer: Value =
        serde_json::from_str(text).map_err(|e| format!("the stale read: {e}: {text}"))?;
    let files = answer["files"].as_array();
    let files = files.ok_or(format!("no files in: {text}"))?;
    let word = |f: &Value| f.as_str().unwrap_or_default().to_string();
    Ok(files.iter().map(word).collect())
}

/// `merge-claim.py --checks`'s rows, `[name, state]`. PENDING and BILLING (GitHub refused to run it) hold no verdict; any
/// other state not green is red. No row yet is no verdict either: a push's runs list within a minute.
fn checks_in(text: &str) -> Result<Checks, String> {
    let rows: Vec<(String, String)> =
        serde_json::from_str(text).map_err(|e| format!("the checks: {e}: {text}"))?;
    let red: Vec<&str> = rows
        .iter()
        .filter(|(_, s)| {
            !GREEN.contains(&s.as_str()) && !matches!(s.as_str(), "PENDING" | "BILLING")
        })
        .map(|(n, _)| n.as_str())
        .collect();
    if !red.is_empty() {
        return Ok(Checks::Red(red.join(", ")));
    }
    let settled = !rows.is_empty() && rows.iter().all(|(_, s)| GREEN.contains(&s.as_str()));
    Ok(if settled {
        Checks::Green
    } else {
        Checks::Pending
    })
}

impl GitHub {
    fn git(&self, args: &[&str]) -> Result<String, String> {
        process::run("git", args, Some(&self.repo), GIT)
    }

    fn gh(&self, args: &[&str]) -> Result<String, String> {
        process::run("gh", args, Some(&self.repo), CALL)
    }

    // ponytail: the merge hook's own rules, never a second copy (dash ci wait reads them too); Rust ones once Python goes. Its
    // other refusals guard one session's merge against another's work, and this path has no second session.
    fn hook(&self, rule: &str, pr: &Pr) -> Result<String, String> {
        let hook = self.repo.join(".claude/hooks/merge-claim.py");
        let number = pr.number.to_string();
        let args = ["-I", &hook.to_string_lossy(), rule, &number];
        process::run("python3", &args, Some(&self.repo), CALL)
    }

    fn folder(&self, id: &str) -> PathBuf {
        self.root.join(format!("rust-{id}"))
    }
}

impl Forge for GitHub {
    fn worktree(&self, id: &str) -> Result<String, String> {
        let folder = self.folder(id);
        if !folder.exists() {
            // Made under another name and moved: a crash half way never leaves a folder a retry would take for a whole one. No
            // session ever ran in what a crash left under that name, so it goes. The claim's lock means only one run is here.
            let new = self.root.join(format!("rust-{id}.new"));
            let path = new.to_string_lossy();
            let _ = self.git(&["worktree", "remove", "--force", &path]);
            if new.exists() {
                std::fs::remove_dir_all(&new).map_err(|e| format!("{path}: {e}"))?;
            }
            self.git(&["worktree", "prune"])?;
            let (branch, base) = (format!("task/{id}"), format!("origin/{}", self.base));
            self.git(&["fetch", "-q", "origin", &self.base])?;
            let known = self.git(&[
                "rev-parse",
                "-q",
                "--verify",
                &format!("refs/heads/{branch}"),
            ]);
            match known {
                Ok(_) => self.git(&["worktree", "add", "-q", &path, &branch])?,
                Err(_) => self.git(&["worktree", "add", "-q", "-b", &branch, &path, &base])?,
            };
            self.git(&["worktree", "move", &path, &folder.to_string_lossy()])?;
        }
        real_folder(&folder)
    }

    fn pr(&self, id: &str) -> Result<Option<Pr>, String> {
        let branch = format!("task/{id}");
        let fields = "number,url,state,headRefOid";
        let args = [
            "pr", "list", "--head", &branch, "--state", "all", "--limit", "1", "--json", fields,
        ];
        let local = self.git(&[
            "-C",
            &self.folder(id).to_string_lossy(),
            "rev-parse",
            "HEAD",
        ]);
        pr_in(&self.gh(&args)?, local.ok().as_deref().map(str::trim))
    }

    fn checks(&self, pr: &Pr) -> Result<Checks, String> {
        checks_in(&self.hook("--checks", pr)?)
    }

    fn stale(&self, pr: &Pr) -> Result<Vec<String>, String> {
        stale_in(&self.hook("--stale", pr)?)
    }

    fn merge(&self, pr: &Pr) -> Result<(), String> {
        let number = pr.number.to_string();
        let args = [
            "pr",
            "merge",
            &number,
            "--squash",
            "--match-head-commit",
            &pr.head,
        ];
        self.gh(&args).map(drop)
    }

    // Its branch goes too, here and at the origin: squashed into main, it holds nothing else, and a task reopened on it would start
    // from an old head that still reads as its merged PR's. Best effort: a reopen on a branch left behind only pushes a merge.
    fn leave(&self, id: &str) -> Result<(), String> {
        let path = self.folder(id);
        self.git(&["worktree", "remove", &path.to_string_lossy()])?;
        let branch = format!("task/{id}");
        let _ = self.git(&["branch", "-D", &branch]);
        let _ = self.git(&["push", "-q", "origin", "--delete", &branch]);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::memory::TestBudget;

    #[test]
    fn a_folder_the_older_app_would_adopt_is_refused_and_another_comes_back_real() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("forge-folder");
        std::fs::create_dir_all(dir.0.join("rust-t/sub")).expect("dirs");
        std::fs::create_dir_all(dir.0.join("task-t/sub")).expect("dirs");
        let real = std::fs::canonicalize(dir.0.join("rust-t")).expect("real");
        assert_eq!(
            real_folder(&dir.0.join("rust-t/")).as_deref(),
            Ok(real.to_string_lossy().as_ref())
        );
        assert!(real_folder(&dir.0.join("task-t")).is_err_and(|e| e.contains("adopt")));
        assert!(real_folder(&dir.0.join("task-t/sub")).is_err_and(|e| e.contains("adopt")));
        assert!(real_folder(&dir.0.join("gone")).is_err_and(|e| e.contains("gone")));
    }

    #[test]
    fn gh_s_answers_read_as_a_pr_that_holds_this_work() {
        let _budget = TestBudget::new();
        assert_eq!(pr_in("[]", None).expect("list"), None);
        let open = r#"[{"number":7,"url":"u","state":"OPEN","headRefOid":"abc"}]"#;
        let pr = pr_in(open, Some("abc")).expect("list").expect("pr");
        assert_eq!(
            (pr.number, pr.url.as_str(), pr.head.as_str(), pr.merged),
            (7, "u", "abc", false)
        );
        assert_eq!(
            pr_in(open, Some("newer")).expect("list"),
            None,
            "not pushed"
        );
        assert!(
            pr_in(open, None).expect("list").is_some(),
            "its worktree gone"
        );
        let merged = r#"[{"number":7,"state":"MERGED","headRefOid":"abc"}]"#;
        assert!(
            pr_in(merged, Some("abc"))
                .expect("list")
                .expect("pr")
                .merged
        );
        assert!(
            pr_in(merged, None).expect("list").expect("pr").merged,
            "its worktree gone"
        );
        assert_eq!(
            pr_in(merged, Some("newer")).expect("list"),
            None,
            "an old one"
        );
        let closed = r#"[{"number":7,"state":"CLOSED","headRefOid":"abc"}]"#;
        assert_eq!(pr_in(closed, Some("abc")).expect("list"), None);
        assert!(pr_in("nope", None).is_err());
    }

    #[test]
    fn the_stale_read_names_the_files() {
        let _budget = TestBudget::new();
        assert_eq!(
            stale_in(r#"{"head":"a","files":[]}"#).expect("read"),
            Vec::<String>::new()
        );
        assert_eq!(
            stale_in(r#"{"head":"a","files":["x"]}"#).expect("read"),
            ["x"]
        );
        assert!(stale_in("{}").is_err());
    }

    #[test]
    fn checks_read_as_the_merge_hook_says() {
        let _budget = TestBudget::new();
        let read = |t: &str| checks_in(t).expect("rows");
        assert_eq!(read(r#"[["a","SUCCESS"],["b","SKIPPED"]]"#), Checks::Green);
        assert_eq!(read("[]"), Checks::Pending);
        assert_eq!(
            read(r#"[["a","SUCCESS"],["b","PENDING"]]"#),
            Checks::Pending
        );
        assert_eq!(read(r#"[["a","BILLING"]]"#), Checks::Pending);
        assert_eq!(
            read(r#"[["a","FAILURE"],["b","PENDING"],["c","NOT_RUN"]]"#),
            Checks::Red("a, c".into())
        );
        assert!(checks_in("nope").is_err());
    }

    /// A real repository, a real worktree: the git half, with no forge in it.
    #[test]
    fn a_worktree_is_made_once_kept_across_a_retry_and_leaves() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("forge-worktree");
        let (origin, repo) = (dir.0.join("origin"), dir.0.join("repo"));
        std::fs::create_dir_all(&origin).expect("dirs");
        let run = |at: &Path, args: &[&str]| process::run("git", args, Some(at), GIT).expect("git");
        run(&origin, &["init", "-q", "-b", "main"]);
        run(
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
        run(
            &dir.0,
            &[
                "clone",
                "-q",
                origin.to_str().expect("path"),
                repo.to_str().expect("path"),
            ],
        );
        let forge = GitHub {
            repo: repo.clone(),
            root: dir.0.clone(),
            base: "main".into(),
        };
        // What a crash half way leaves: a half-made folder, with the branch checked out there, then another with no worktree.
        let new = dir.0.join("rust-t1.new");
        run(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "task/t1",
                new.to_str().expect("path"),
                "origin/main",
            ],
        );
        std::fs::remove_file(new.join(".git")).expect("half made");
        let folder = forge.worktree("t1").expect("made");
        assert!(folder.ends_with("rust-t1"));
        assert!(!new.exists(), "its working name is gone");
        forge.leave("t1").expect("left");
        std::fs::create_dir_all(new.join("stray")).expect("half made");
        let folder = forge.worktree("t1").expect("made");
        assert!(!new.exists());
        let branch = run(Path::new(&folder), &["branch", "--show-current"]);
        assert_eq!(branch.trim(), "task/t1");
        std::fs::write(Path::new(&folder).join("left"), "x").expect("a run's work");
        assert_eq!(forge.worktree("t1").expect("kept"), folder);
        assert!(Path::new(&folder).join("left").exists());
        assert!(forge.leave("t1").is_err(), "git refuses a dirty one");
        std::fs::remove_file(Path::new(&folder).join("left")).expect("clean");
        forge.leave("t1").expect("left");
        assert!(!Path::new(&folder).exists());
        // A reopen: its old commit, merged and gone, is no head it starts from, so no old PR reads as holding its work.
        let again = forge.worktree("t1").expect("again");
        assert_eq!(again, folder);
        run(
            Path::new(&again),
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "merged since",
            ],
        );
        let old = run(Path::new(&again), &["rev-parse", "HEAD"]);
        forge.leave("t1").expect("left");
        let reopened = forge.worktree("t1").expect("reopened");
        let head = run(Path::new(&reopened), &["rev-parse", "HEAD"]);
        assert_ne!(head, old);
        assert_eq!(head, run(&repo, &["rev-parse", "origin/main"]));
    }
}
