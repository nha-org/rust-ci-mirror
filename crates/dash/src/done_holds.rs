//! Nothing finished holds anything (task rust-nothing-done-holds; @nha, 2026-11-12: « how can we have less defects like this
//! one ? »): 9 merged tasks kept their worktree leases while only « held by nobody » was checked, never its mirror. Here every
//! kind a task holds is read against the list: a worktree (its lease), a Rust claim, a run's PR hold, and a herdr pane working in
//! its worktree. `dash orch --inconsistencies` names each one a finished task holds; the hourly reap releases each that nothing
//! works in, and says so, whichever path forgot it (its run's merge, `dash task close`, a close by hand).
//! ponytail: a hold of the orchestrator or the lane names no task, so it is left to its own timeout (`HOLD_MS`, `LANE_HOLD_MS`).
use dash_core::tasks::{Claim, Forge, Hold, Outcome, Run, Standing, TaskStore, Terminal};
use dash_terminal::Live;
use std::path::{Path, PathBuf};

/// What a finished task may still hold, read once a pass.
pub struct Held {
    /// Each task the list holds done or dropped, and how: « merged » when its Rust run's PR merged, else « closed ».
    pub done: Vec<(String, &'static str)>,
    /// The tasks with a worktree `rust-<id>` in `root`.
    pub worktrees: Vec<String>,
    pub claims: Vec<Claim>,
    /// Of `claims`, those whose run no longer lives (`TaskStore::crashed`). The hook reads none: naming asks no lock.
    pub crashed: Vec<String>,
    pub holds: Vec<Hold>,
    /// Every agent herdr shows, of any project.
    pub panes: Vec<Live>,
    /// The folder the task worktrees sit in.
    pub root: PathBuf,
    /// The claims or herdr's agents unread: no hold ends and no worktree goes, as a run or a session may work there.
    pub blind: bool,
}

/// One thing a finished task holds.
#[derive(Debug, PartialEq)]
enum Kind {
    Pane { pane: String, status: String },
    Claim,
    Hold(u64),
    Lease,
}

struct Found<'a> {
    id: &'a str,
    how: &'static str,
    kind: Kind,
}

impl Held {
    fn how(&self, id: &str) -> Option<&'static str> {
        self.done.iter().find(|(d, _)| d == id).map(|(_, how)| *how)
    }

    fn task_at<'s>(&self, cwd: &'s str) -> Option<&'s str> {
        task_at(&self.root, cwd)
    }

    /// Each thing a finished task holds, panes first, then claims, holds and worktrees: the order `release` frees them in, so
    /// a worktree is judged once what worked in it is gone.
    fn found(&self) -> Vec<Found<'_>> {
        let mut found = vec![];
        for live in &self.panes {
            let Some(id) = self.task_at(&live.cwd) else {
                continue;
            };
            if let Some(how) = self.how(id) {
                let (pane, status) = (live.pane.clone(), live.status.clone());
                found.push(Found {
                    id,
                    how,
                    kind: Kind::Pane { pane, status },
                });
            }
        }
        for claim in &self.claims {
            if let Some(how) = self.how(&claim.id) {
                found.push(Found {
                    id: &claim.id,
                    how,
                    kind: Kind::Claim,
                });
            }
        }
        for hold in &self.holds {
            let id = hold.holder.strip_prefix("run ");
            if let Some((id, how)) = id.and_then(|id| Some((id, self.how(id)?))) {
                found.push(Found {
                    id,
                    how,
                    kind: Kind::Hold(hold.pr),
                });
            }
        }
        for id in &self.worktrees {
            if let Some(how) = self.how(id) {
                found.push(Found {
                    id,
                    how,
                    kind: Kind::Lease,
                });
            }
        }
        found
    }

    fn named(&self, f: &Found) -> String {
        match &f.kind {
            Kind::Pane { pane, status } => format!("pane {pane} ({status})"),
            Kind::Claim => "its claim".into(),
            Kind::Hold(pr) => format!("a hold on PR {pr}"),
            Kind::Lease => format!(
                "its worktree {}",
                self.root.join(format!("rust-{}", f.id)).display()
            ),
        }
    }

    /// `dash orch --inconsistencies`' lines: each thing a finished task holds.
    pub fn lines(&self) -> Vec<String> {
        let found = self.found();
        let line = |f: &Found| format!("task {} is {} but holds {}", f.id, f.how, self.named(f));
        found.iter().map(line).collect()
    }

    /// The reap's pass: each thing a finished task holds released when nothing works in it, each said, released or kept. A pane
    /// whose agent is idle or done is closed, one working or asking kept; a claim whose run lives is kept, a crashed one ends as
    /// its task did, the list not told again; a run's hold ends unless its claim stays; a worktree goes once no claim or pane is
    /// left in it (`Forge::release_done`, which keeps what no remote has).
    pub fn release(
        &self,
        tasks: &mut dyn TaskStore,
        forge: &dyn Forge,
        terminal: &dyn Terminal,
    ) -> Vec<String> {
        let mut busy: Vec<&str> = vec![];
        let mut said = vec![];
        for f in self.found() {
            let what = self.named(&f);
            let kept = |why: &str| format!("{}: {}, {what} kept: {why}", f.id, f.how);
            let went = |how: &str| format!("{}: {}, {what} {how}", f.id, f.how);
            let words = match &f.kind {
                Kind::Pane { pane, status } => match status.as_str() {
                    "idle" | "done" if dash_comments::fails::running(terminal, pane) => {
                        Err(format!("pane {pane} shows a running tool call"))
                    }
                    "idle" | "done" => terminal.close(pane).map(|()| went("closed")),
                    _ => Err(format!("its agent is {status}")),
                },
                Kind::Claim if !self.crashed.iter().any(|c| c == f.id) => {
                    Err("its run lives".into())
                }
                Kind::Claim if busy.contains(&f.id) => Err("its pane is still open".into()),
                Kind::Claim if let Some(pane) = self.running_pane(f.id, terminal) => {
                    Err(format!("pane {pane} shows a running tool call"))
                }
                Kind::Claim => self
                    .end_claim(f.id, f.how, tasks)
                    .map(|()| went("released, its run gone")),
                Kind::Hold(_) | Kind::Lease if self.blind => {
                    Err("the claims or panes are unread".into())
                }
                Kind::Hold(_) if busy.contains(&f.id) => Err("its claim stays".into()),
                Kind::Hold(pr) => tasks.unhold(*pr).map(|()| went("ended")),
                Kind::Lease if busy.contains(&f.id) => Err("a claim or pane stays in it".into()),
                Kind::Lease => forge
                    .release_done(f.id)
                    .map(|w| format!("{}: {}, {what}: {w}", f.id, f.how)),
            };
            said.push(words.unwrap_or_else(|why| {
                busy.push(f.id);
                kept(&why)
            }));
        }
        said
    }

    /// The claim's pane, or a pane in its task's worktree, whose screen shows a running tool call: herdr may read a run as
    /// done or gone while its tool runs (task pane-fail-detect), so a release waits on the screen too.
    fn running_pane(&self, id: &str, terminal: &dyn Terminal) -> Option<String> {
        let claimed = self
            .claims
            .iter()
            .filter(|c| c.id == id)
            .filter_map(|c| c.pane.clone());
        let here = (self.panes.iter())
            .filter(|p| self.task_at(&p.cwd) == Some(id))
            .map(|p| p.pane.clone());
        claimed
            .chain(here)
            .find(|p| dash_comments::fails::running(terminal, p))
    }

    /// A crashed claim of a finished task: held as an adoption holds one, so no run takes it meanwhile, then recorded ended and
    /// told, as the list already holds it finished.
    fn end_claim(&self, id: &str, how: &str, tasks: &mut dyn TaskStore) -> Result<(), String> {
        let claim = tasks.hold(id)?;
        let words = format!("its task {how}: its claim released at the reap, its run gone");
        let run = Run {
            pane: claim.pane,
            session: claim.session,
            outcome: Outcome::Ended(words),
        };
        tasks.record(id, &run)?;
        tasks.told(id)
    }
}

/// How each of `ids` the list holds closed is finished: « merged » when its Rust row's run merged, else « closed ».
pub fn done_of(closed: &[String], rows: &[Standing]) -> Vec<(String, &'static str)> {
    let merged = |id: &str| {
        rows.iter()
            .any(|r| r.id == id && r.outcome.as_deref() == Some("merged"))
    };
    let how = |id: &String| (id.clone(), if merged(id) { "merged" } else { "closed" });
    closed.iter().map(how).collect()
}

/// The task whose worktree in `root` (as given or real: herdr reads a pane's folder real) holds `cwd`, at its top or below.
fn task_at<'s>(root: &Path, cwd: &'s str) -> Option<&'s str> {
    let name = dash_forge::folder::below(cwd, root)?.iter().next()?;
    dash_forge::task_folder(name.to_str()?)
}

/// What the hook and the reap read: Rust's store, the worktrees' folder, herdr's agents and the list's closed tasks. `crashed`:
/// the reap asks which claims' runs are gone, which takes each claim's lock a moment; the hook only names, so asks none. Each
/// part unread is its own line, and a list unread judges nothing finished. `root`: the worktrees' folder; `list`: the older app's
/// database, which holds the list.
pub fn read(
    tasks: &dyn TaskStore,
    crashed: bool,
    root: &Path,
    list: &Path,
    panes: Result<Vec<Live>, String>,
) -> (Held, Vec<String>) {
    let mut unread = vec![];
    fn or_none<T>(unread: &mut Vec<String>, part: &str, read: Result<Vec<T>, String>) -> Vec<T> {
        read.unwrap_or_else(|e| {
            unread.push(format!("finished tasks' holds: {part} unread: {e}"));
            vec![]
        })
    }
    let root = root.to_path_buf();
    let entries = std::fs::read_dir(&root).map_err(|e| format!("{}: {e}", root.display()));
    let names = entries.map(|e| {
        e.flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().into_string().ok())
    });
    let worktrees: Vec<String> = or_none(&mut unread, "worktrees", names.map(|n| n.collect()))
        .into_iter()
        .filter_map(|n: String| dash_forge::task_folder(&n).map(String::from))
        .collect();
    let before = unread.len();
    let claims = or_none(&mut unread, "claims", tasks.claims());
    let mut blind = unread.len() > before;
    let crashed = match crashed {
        true => or_none(&mut unread, "crashed claims", tasks.crashed())
            .into_iter()
            .map(|c| c.id)
            .collect(),
        false => vec![],
    };
    let holds = or_none(&mut unread, "holds", tasks.holds());
    let rows = or_none(&mut unread, "rows", tasks.standing());
    let before = unread.len();
    let panes = or_none(&mut unread, "panes", panes);
    blind |= unread.len() > before;
    let mut ids: Vec<String> = worktrees.clone();
    ids.extend(claims.iter().map(|c| c.id.clone()));
    ids.extend(
        holds
            .iter()
            .filter_map(|h| h.holder.strip_prefix("run ").map(String::from)),
    );
    ids.extend(
        panes
            .iter()
            .filter_map(|p| task_at(&root, &p.cwd).map(String::from)),
    );
    ids.sort();
    ids.dedup();
    let places = dash_store::older::places(list, &ids);
    let closed = places.map(|p| {
        p.into_iter()
            .filter(|(_, p)| p.closed)
            .map(|(id, _)| id)
            .collect()
    });
    let done = done_of(&or_none(&mut unread, "the list", closed), &rows);
    let held = Held {
        done,
        worktrees,
        claims,
        crashed,
        holds,
        panes,
        root,
        blind,
    };
    (held, unread)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_core::tasks::{Base, Checks, Pr, Seen, Serving, run_holder};
    use dash_meter::{TestDir, dash_test};
    use dash_store::tasks::SqliteTasks;
    use std::cell::RefCell;

    const ROOT: &str = "/w/site_perso";

    /// What the reap asked of the forge and the terminal.
    #[derive(Default)]
    struct Asked {
        released: RefCell<Vec<String>>,
        closed: RefCell<Vec<String>>,
        /// Each pane's screen; a pane not here does not read.
        screens: std::collections::BTreeMap<String, String>,
    }

    impl Forge for Asked {
        fn release_done(&self, id: &str) -> Result<String, String> {
            self.released.borrow_mut().push(id.into());
            Ok("worktree released".into())
        }
        fn worktree(&self, _: &str) -> Result<String, String> {
            unreachable!()
        }
        fn pr(&self, _: &str) -> Result<Option<Pr>, String> {
            unreachable!()
        }
        fn checks(&self, _: &Pr) -> Result<Checks, String> {
            unreachable!()
        }
        fn base(&self, _: &Pr) -> Result<Base, String> {
            unreachable!()
        }
        fn merge(&self, _: &Pr) -> Result<(), String> {
            unreachable!()
        }
        fn update(&self, _: &str, _: &Pr) -> Result<(), String> {
            unreachable!()
        }
        fn leave(&self, _: &str) -> Result<(), String> {
            unreachable!()
        }
        fn restack(&self, _: &Pr, _: &[String]) -> Result<Vec<String>, String> {
            unreachable!()
        }
        fn clean(&self, _: &str) -> Result<(), String> {
            unreachable!()
        }
        fn deploy(&self) -> Result<String, String> {
            unreachable!()
        }
        fn serving(&self) -> Result<Serving, String> {
            unreachable!()
        }
        fn save_monitors(&self) -> Result<Option<String>, String> {
            unreachable!()
        }
        fn diff(&self, _: &str) -> Result<String, String> {
            unreachable!()
        }
    }

    impl Terminal for Asked {
        fn close(&self, pane: &str) -> Result<(), String> {
            self.closed.borrow_mut().push(pane.into());
            Ok(())
        }
        fn open(&self, _: &str, _: &str) -> Result<String, String> {
            unreachable!()
        }
        fn run(&self, _: &str, _: &str) -> Result<(), String> {
            unreachable!()
        }
        fn agent(&self, _: &str) -> Result<Option<Seen>, String> {
            unreachable!()
        }
        fn sessions(&self) -> Result<usize, String> {
            unreachable!()
        }
        fn read(&self, pane: &str) -> Result<String, String> {
            self.screens
                .get(pane)
                .cloned()
                .ok_or(format!("{pane}: no such pane"))
        }
        fn send(&self, _: &str, _: &str) -> Result<(), String> {
            unreachable!()
        }
        fn key(&self, _: &str, _: &str) -> Result<(), String> {
            unreachable!()
        }
    }

    fn pane(pane: &str, status: &str, cwd: &str) -> Live {
        let (pane, status, cwd) = (pane.into(), status.into(), cwd.into());
        Live {
            pane,
            title: "t".into(),
            status,
            cwd,
        }
    }

    fn claim(id: &str) -> Claim {
        Claim {
            id: id.into(),
            pane: None,
            session: None,
            attempts: 1,
            account: None,
            model: None,
        }
    }

    fn hold(pr: u64, holder: &str) -> Hold {
        Hold {
            pr,
            holder: holder.into(),
            why: "its run lands it".into(),
            touched_ms: 0,
        }
    }

    fn held(done: &[(&str, &'static str)]) -> Held {
        Held {
            done: done
                .iter()
                .map(|(id, how)| (id.to_string(), *how))
                .collect(),
            worktrees: vec![],
            claims: vec![],
            crashed: vec![],
            holds: vec![],
            panes: vec![],
            root: ROOT.into(),
            blind: false,
        }
    }

    /// A store in `dir` whose rows `ids` are claimed by a run gone: its lock free, as a crash leaves it.
    fn crashed_store(dir: &TestDir, ids: &[&str]) -> SqliteTasks {
        let locks = dir.0.join("locks");
        std::fs::create_dir_all(&locks).expect("dirs");
        let mut first = dash_store::tasks::open(&dir.0, &locks).expect("a store");
        for id in ids {
            first.claim(id).expect("claimed");
        }
        drop(first);
        dash_store::tasks::open(&dir.0, &locks).expect("the store again")
    }

    /// Done-when 1: each lease, claim, hold or pane a finished task holds is named, merged or closed; an open task's never, nor
    /// a pane outside the worktrees, a half-made `.new` folder's, or a hold the orchestrator took.
    #[dash_test]
    fn each_lease_claim_hold_or_pane_a_finished_task_holds_is_named() {
        let mut h = held(&[("rust-a", "merged"), ("rust-b", "closed")]);
        h.worktrees = vec!["rust-a".into(), "rust-open".into()];
        h.claims = vec![claim("rust-b"), claim("rust-open")];
        h.holds = vec![
            hold(1401, &run_holder("rust-a")),
            hold(1402, "orchestrator"),
            hold(1403, "run rust-open"),
        ];
        h.panes = vec![
            pane("w1:p1", "idle", "/w/site_perso/rust-rust-b/rust"),
            pane("w2:p1", "working", "/w/site_perso/rust-rust-open"),
            pane("w3:p1", "idle", "/elsewhere/rust-rust-a"),
            pane("w4:p1", "idle", "/w/site_perso/rust-rust-a.new"),
        ];
        assert_eq!(
            h.lines(),
            [
                "task rust-b is closed but holds pane w1:p1 (idle)",
                "task rust-b is closed but holds its claim",
                "task rust-a is merged but holds a hold on PR 1401",
                "task rust-a is merged but holds its worktree /w/site_perso/rust-rust-a",
            ]
        );
        let how = done_of(
            &["rust-a".into(), "rust-b".into()],
            &[Standing {
                id: "rust-a".into(),
                status: "finished".into(),
                outcome: Some("merged".into()),
                ..Default::default()
            }],
        );
        assert_eq!(
            how,
            [
                ("rust-a".to_string(), "merged"),
                ("rust-b".to_string(), "closed")
            ]
        );
    }

    /// Done-when 2 (pane): a finished task's pane whose agent is idle or done is closed at the reap; one working or asking him
    /// a question is kept, said, and holds its worktree.
    #[dash_test]
    fn a_finished_task_s_idle_pane_is_closed_at_the_reap_a_working_one_kept() {
        let dir = TestDir::new("done-holds-pane");
        let mut tasks = crashed_store(&dir, &[]);
        let mut h = held(&[
            ("rust-a", "merged"),
            ("rust-b", "merged"),
            ("rust-c", "closed"),
        ]);
        h.panes = vec![
            pane("w1:p1", "done", "/w/site_perso/rust-rust-a"),
            pane("w2:p1", "working", "/w/site_perso/rust-rust-b"),
            pane("w3:p1", "blocked", "/w/site_perso/rust-rust-c"),
        ];
        h.worktrees = vec!["rust-a".into(), "rust-b".into()];
        let asked = Asked::default();
        let said = h.release(&mut tasks, &asked, &asked);
        assert_eq!(
            said,
            [
                "rust-a: merged, pane w1:p1 (done) closed",
                "rust-b: merged, pane w2:p1 (working) kept: its agent is working",
                "rust-c: closed, pane w3:p1 (blocked) kept: its agent is blocked",
                "rust-a: merged, its worktree /w/site_perso/rust-rust-a: worktree released",
                "rust-b: merged, its worktree /w/site_perso/rust-rust-b kept: a claim or pane stays in it",
            ]
        );
        assert_eq!(*asked.closed.borrow(), ["w1:p1"]);
        assert_eq!(*asked.released.borrow(), ["rust-a"]);
    }

    /// Done-when 2 (claim): a finished task's claim whose run is gone ends at the reap, recorded ended and never told to the list
    /// again; one whose run lives is kept, said, with its hold and worktree.
    #[dash_test]
    fn a_finished_task_s_crashed_claim_ends_at_the_reap_a_live_run_s_kept() {
        let dir = TestDir::new("done-holds-claim");
        let mut tasks = crashed_store(&dir, &["rust-a", "rust-b"]);
        let mut h = held(&[("rust-a", "closed"), ("rust-b", "merged")]);
        h.claims = TaskStore::claims(&tasks).expect("claims");
        // rust-b's run lives: the reap's read of `crashed` leaves it out
        h.crashed = vec!["rust-a".into()];
        h.holds = vec![hold(1523, &run_holder("rust-b"))];
        h.worktrees = vec!["rust-a".into(), "rust-b".into()];
        let asked = Asked::default();
        let said = h.release(&mut tasks, &asked, &asked);
        assert_eq!(
            said,
            [
                "rust-a: closed, its claim released, its run gone",
                "rust-b: merged, its claim kept: its run lives",
                "rust-b: merged, a hold on PR 1523 kept: its claim stays",
                "rust-a: closed, its worktree /w/site_perso/rust-rust-a: worktree released",
                "rust-b: merged, its worktree /w/site_perso/rust-rust-b kept: a claim or pane stays in it",
            ]
        );
        let ids: Vec<String> = TaskStore::claims(&tasks)
            .expect("claims")
            .into_iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(ids, ["rust-b"], "only the live run's claim stays");
        assert_eq!(
            tasks.status("rust-a").expect("row").as_deref(),
            Some("finished")
        );
        assert_eq!(
            tasks.untold("rust-a").expect("told"),
            None,
            "the list, which closed it, is not told again"
        );
    }

    /// Task pane-fail-detect, done-when 5: a crashed claim whose pane shows a running tool call is kept, naming the pane, and
    /// so is a pane herdr calls done while its tool runs; the same claim with a quiet screen ends.
    #[dash_test]
    fn a_claim_whose_pane_shows_a_running_tool_call_is_not_released() {
        let dir = TestDir::new("done-holds-running");
        let mut tasks = crashed_store(&dir, &["rust-a", "rust-b"]);
        let mut h = held(&[("rust-a", "closed"), ("rust-b", "closed")]);
        h.claims = TaskStore::claims(&tasks).expect("claims");
        h.claims[0].pane = Some("w5:p1".into());
        h.crashed = vec!["rust-a".into(), "rust-b".into()];
        h.panes = vec![pane("w6:p1", "done", "/w/site_perso/rust-rust-b")];
        let running = "TASK_DONE\n⏺ Bash(cargo nextest run)\n  ⎿  Running…\n";
        let asked = Asked {
            screens: [("w5:p1", running), ("w6:p1", running)]
                .into_iter()
                .map(|(p, s)| (p.to_string(), s.to_string()))
                .collect(),
            ..Asked::default()
        };
        let said = h.release(&mut tasks, &asked, &asked);
        assert_eq!(
            said,
            [
                "rust-b: closed, pane w6:p1 (done) kept: pane w6:p1 shows a running tool call",
                "rust-a: closed, its claim kept: pane w5:p1 shows a running tool call",
                "rust-b: closed, its claim kept: its pane is still open",
            ]
        );
        assert!(asked.closed.borrow().is_empty(), "no pane closed");
        let quiet = Asked::default();
        let said = h.release(&mut tasks, &quiet, &quiet);
        assert_eq!(
            said[1], "rust-a: closed, its claim released, its run gone",
            "{said:?}"
        );
    }

    /// Done-when 2 (hold): a finished task's run hold ends at the reap once no claim of it stays, the orchestrator's untouched.
    #[dash_test]
    fn a_finished_task_s_run_hold_ends_at_the_reap() {
        let dir = TestDir::new("done-holds-hold");
        let mut tasks = crashed_store(&dir, &[]);
        tasks
            .hold_pr(1523, &run_holder("rust-a"), "its run lands it")
            .expect("held");
        tasks
            .hold_pr(1524, "orchestrator", "dash land")
            .expect("held");
        let mut h = held(&[("rust-a", "merged")]);
        h.holds = TaskStore::holds(&tasks).expect("holds");
        let asked = Asked::default();
        let said = h.release(&mut tasks, &asked, &asked);
        assert_eq!(said, ["rust-a: merged, a hold on PR 1523 ended"]);
        let left: Vec<u64> = TaskStore::holds(&tasks)
            .expect("holds")
            .into_iter()
            .map(|h| h.pr)
            .collect();
        assert_eq!(left, [1524]);
    }

    /// Done-when 2 (lease): a finished task's worktree nothing works in is released at the reap, an open task's never; the
    /// forge's own test (`a_finished_task_s_worktree_goes_after_an_hour_when_nothing_works_in_it`) proves what it keeps.
    #[dash_test]
    fn a_finished_task_s_worktree_is_released_at_the_reap() {
        let dir = TestDir::new("done-holds-lease");
        let mut tasks = crashed_store(&dir, &[]);
        let mut h = held(&[("rust-a", "merged")]);
        h.worktrees = vec!["rust-a".into(), "rust-open".into()];
        let asked = Asked::default();
        let said = h.release(&mut tasks, &asked, &asked);
        assert_eq!(
            said,
            ["rust-a: merged, its worktree /w/site_perso/rust-rust-a: worktree released"]
        );
        assert_eq!(*asked.released.borrow(), ["rust-a"]);
        // herdr or the claims unread: a session or a run may work there, so it stays, and so does a run's hold
        h.blind = true;
        h.holds = vec![hold(1523, &run_holder("rust-a"))];
        let said = h.release(&mut tasks, &asked, &asked);
        let kept = "kept: the claims or panes are unread";
        assert!(
            said.len() == 2 && said.iter().all(|s| s.ends_with(kept)),
            "{said:?}"
        );
        assert_eq!(asked.released.borrow().len(), 1, "nothing more released");
    }

    /// /code-review of rust-worktrees-noindex: herdr reads a pane's folder real, so a pane under the linked worktrees' real path
    /// is still its task's.
    #[dash_test]
    fn a_pane_at_the_worktrees_real_path_is_its_task_s() {
        let dir = TestDir::new("done-holds-real-path");
        let real = dir.0.join("worktrees.noindex/.herdr/worktrees");
        std::fs::create_dir_all(real.join("rust-rust-a/rust")).expect("dirs");
        let link = dir.0.join("worktrees");
        std::os::unix::fs::symlink(&real, &link).expect("a link");
        let at = std::fs::canonicalize(real.join("rust-rust-a/rust")).expect("real");
        assert_eq!(task_at(&link, &at.to_string_lossy()), Some("rust-a"));
        assert_eq!(
            task_at(&link, &dir.0.join("elsewhere").to_string_lossy()),
            None
        );
    }

    /// The read path: worktrees as the forge names them, the claims from the store, and herdr unread made blind, so the reap
    /// then releases no hold or worktree; a list unread judges nothing finished. Each part unread is said (/code-review of
    /// 9a61ee07f: the read was untested).
    #[dash_test]
    fn a_read_names_the_forge_s_worktrees_and_goes_blind_without_herdr() {
        let dir = TestDir::new("done-holds-read");
        let tasks = crashed_store(&dir, &["rust-a"]);
        let root = dir.0.join("worktrees");
        for folder in ["rust-rust-a", "rust-rust-b.new", "pool.noindex"] {
            std::fs::create_dir_all(root.join(folder)).expect("dirs");
        }
        std::fs::write(root.join("rust-rust-c.lock"), "1").expect("a lock");
        let no_list = dir.0.join("no-list.db");
        let (held, unread) = read(&tasks, true, &root, &no_list, Err("herdr is down".into()));
        assert_eq!(held.worktrees, ["rust-a"]);
        assert_eq!(held.claims.len(), 1);
        assert_eq!(held.crashed, ["rust-a"]);
        assert!(
            held.blind,
            "herdr unread: a session may work in any worktree"
        );
        assert!(
            held.done.is_empty(),
            "a list unread judges nothing finished"
        );
        assert_eq!(unread.len(), 2, "{unread:?}");
        assert_eq!(
            unread[0],
            "finished tasks' holds: panes unread: herdr is down"
        );
        assert!(unread[1].starts_with("finished tasks' holds: the list unread: "));
        let (held, _) = read(&tasks, false, &root, &no_list, Ok(vec![]));
        assert!(
            !held.blind && held.crashed.is_empty(),
            "the hook asks no lock"
        );
    }
}
