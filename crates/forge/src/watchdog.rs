//! The VPS runner's watchdog (task rust-vps-runner-watchdog; @nha, 2026-11-11: « Think about the system and what would
//! improve it for the future. Spike, measure. »): each pass of `dash orch` reads the repository's running and queued runs
//! and looks for two kinds that hold the queue of the runner labelled [`LABEL`]:
//! - **Overdue:** a job on that label still in progress [`GRACE_MIN`] minutes past its workflow's `timeout-minutes` is
//!   cancelled; [`GRACE_MIN`] more and the cancel is forced. GitHub enforces the limit only through a runner that hears it:
//!   on 2026-11-11 the VPS runner's broker poll stalled from 20:17:51 to 20:56:52 (its _diag), and GitHub kept run
//!   43095022385's finished job « in progress » about 40 minutes against its 6.
//! - **Ghost:** a run of a workflow that runs on that label, queued [`GHOST_MIN`] minutes with no job at all, is named, never
//!   ended: GitHub dropped its job (the stalled runner's acknowledge at 20:56:52 answered « Job not found »), so it never
//!   starts and holds no runner, yet /status's CI queue counts it. Run 43095721996 waited so from 20:23; read at 21:33 as
//!   « 63 minutes on the VPS runner », it had the orchestrator cancel run 43104132006, a live gate 27 seconds old. GitHub
//!   refuses every end of it: both cancels (409, « Cannot cancel a workflow run that has not been queued yet ») and the
//!   delete (403), 2026-11-11, so a pass asks nothing more of it. Its pull request's gate runs again on its next push.
//!
//! What a pass ends and every ghost are named in its line, which /watchers shows (the dash crate's `watchdog`). Every call
//! goes through `ask` (`gh` and its words), so a test stands in for the forge.
use crate::runs::unix;

/// The label of the VPS runners (bin/ci-runner vps).
pub const LABEL: &str = "site-perso-vps";
/// Minutes past a job's own limit before it is cancelled, and again before the cancel is forced.
pub const GRACE_MIN: i64 = 5;
/// Minutes a run waits with no job before it counts as dropped: GitHub creates a run's jobs within seconds.
pub const GHOST_MIN: i64 = 15;
/// The limit of a workflow that states none as a number: dashboard.yml's VPS route takes 45 (its timeout-minutes expression).
pub const DEFAULT_LIMIT_MIN: i64 = 45;

/// A workflow file's limit in minutes: the largest `timeout-minutes` it holds, an expression counting as
/// [`DEFAULT_LIMIT_MIN`] (dashboard.yml's VPS route is an expression giving 45 beside its gate job's literal 6), and with none,
/// [`DEFAULT_LIMIT_MIN`].
// ponytail: the largest of the file, not the VPS job's own; each VPS workflow holds one job with a number today
pub fn limit_of(workflow: &str) -> i64 {
    workflow
        .lines()
        .filter_map(|l| l.trim().strip_prefix("timeout-minutes:"))
        // a comment or quotes around the number are YAML's, not an expression (/code-review)
        .map(|v| {
            v.split('#')
                .next()
                .unwrap_or(v)
                .trim()
                .trim_matches(['\'', '"'])
        })
        .map(|v| v.parse::<i64>().unwrap_or(DEFAULT_LIMIT_MIN))
        .max()
        .unwrap_or(DEFAULT_LIMIT_MIN)
}

/// One pass at `now` (Unix seconds) on `slug`: the line for /watchers, one line for each run it ended or was refused, and
/// whether the pass is OK: false once GitHub refused an end or a running run did not read, since either may hide a hold. Each
/// run's workflow is read at its own head commit, so a branch that changes its limit or its runner is judged by its own file.
/// A run whose jobs or workflow do not read is named « unread » and the pass goes on.
pub fn pass(
    ask: &dyn Fn(&[&str]) -> Result<String, String>,
    slug: &str,
    now: i64,
) -> Result<(String, Vec<String>, bool), String> {
    let runs = |status: &str| -> Result<Vec<(String, String, String, i64)>, String> {
        let path = format!("repos/{slug}/actions/runs?status={status}&per_page=100");
        let jq = ".workflow_runs[] | [.id, .path, .head_sha, .created_at] | @tsv";
        Ok(ask(&["api", &path, "--jq", jq])?
            .lines()
            .filter_map(|row| match row.split('\t').collect::<Vec<_>>()[..] {
                [id, path, sha, created] => {
                    Some((id.into(), path.into(), sha.into(), unix(created)?))
                }
                _ => None,
            })
            .collect())
    };
    let workflow = |path: &str, sha: &str| {
        let at = format!("repos/{slug}/contents/{path}?ref={sha}");
        ask(&["api", "-H", "Accept: application/vnd.github.raw", &at])
    };
    let jobs = |id: &str, jq: &str| {
        ask(&[
            "api",
            &format!("repos/{slug}/actions/runs/{id}/jobs"),
            "--jq",
            jq,
        ])
    };
    // a refusal is named and the pass goes on, so one run GitHub will not end never hides the others
    let end = |id: &str, how: &str, done: String| {
        let path = format!("repos/{slug}/actions/runs/{id}/{how}");
        match ask(&["api", "-X", "POST", &path]) {
            Ok(_) => done,
            Err(e) => format!("run {id}: {how} refused: {e}"),
        }
    };
    let name = |path: &str| path.rsplit('/').next().unwrap_or(path).to_string();
    let (running, queued) = (runs("in_progress")?, runs("queued")?);
    let (mut ended, mut notes, mut ok) = (vec![], vec![], true);
    let jq = ".jobs[] | [.name, .status, (.labels | join(\",\")), (.started_at // \"\")] | @tsv";
    for (id, path, sha, _) in &running {
        let rows = match jobs(id, jq) {
            Ok(rows) => rows,
            Err(e) => {
                notes.push(format!("run {id}: jobs unread: {e}"));
                ok = false;
                continue;
            }
        };
        // the oldest job in progress on the label: the only one read further
        let Some((job, started)) = rows
            .lines()
            .filter_map(|row| match row.split('\t').collect::<Vec<_>>()[..] {
                [job, "in_progress", labels, started] if labels.split(',').any(|l| l == LABEL) => {
                    Some((job.to_string(), unix(started)?))
                }
                _ => None,
            })
            .min_by_key(|(_, started)| *started)
        else {
            continue;
        };
        // no job runs past the shortest limit plus the grace without a read of its own file
        if (now - started) / 60 < GRACE_MIN {
            continue;
        }
        let limit = match workflow(path, sha) {
            Ok(w) => limit_of(&w),
            Err(e) => {
                notes.push(format!("run {id}: {} unread: {e}", name(path)));
                ok = false;
                continue;
            }
        };
        let past = (now - started) / 60 - limit;
        if past < GRACE_MIN {
            continue;
        }
        let how = if past < 2 * GRACE_MIN {
            "cancel"
        } else {
            "force-cancel"
        };
        let done = format!(
            "run {id} ({} {job}) {how}led: {past} min past its {limit}-minute limit",
            name(path)
        );
        let said = end(id, how, done);
        ok &= !said.contains(" refused: ");
        ended.push(said);
    }
    for (id, path, sha, created) in &queued {
        let waited = (now - created) / 60;
        if waited < GHOST_MIN {
            continue;
        }
        let count = match jobs(id, ".total_count") {
            Ok(c) => c,
            Err(e) => {
                notes.push(format!("run {id}: jobs unread: {e}"));
                continue;
            }
        };
        // a queued run holds no runner, so an unread one is named but never fails the pass: it may be the Mac's
        if count.trim() != "0" {
            continue;
        }
        match workflow(path, sha) {
            Ok(w) if w.contains(LABEL) => {}
            Ok(_) => continue,
            Err(e) => {
                notes.push(format!(
                    "run {id}: queued {waited} min with no job, {} unread: {e}",
                    name(path)
                ));
                continue;
            }
        }
        notes.push(format!(
            "run {id} ({}) queued {waited} min with no job: GitHub dropped it, it never starts and holds no runner, so no \
             running job is hung for it; its PR's gate runs again on its next push",
            name(path)
        ));
    }
    let mut line = format!(
        "{LABEL}: {} running, {} queued; ",
        running.len(),
        queued.len()
    );
    line += &if ended.is_empty() && notes.is_empty() {
        "nothing overdue, no run without a job".to_string()
    } else {
        [&ended[..], &notes[..]].concat().join("; ")
    };
    Ok((line, ended, ok))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;
    use std::cell::RefCell;

    const GATE: &str = "jobs:\n  gate:\n    runs-on: [site-perso-vps]\n    timeout-minutes: 6\n";
    const MAC: &str =
        "jobs:\n  check:\n    runs-on: [self-hosted, site-perso]\n    timeout-minutes: 40\n";
    /// 2026-11-11T20:17:00Z, run 43095022385's job start.
    const START: i64 = 1_794_428_220;

    /// A stand-in forge: `running` and `queued` answer the two listings, `jobs` each run's jobs by id; every call is kept.
    struct Forge {
        running: String,
        queued: String,
        jobs: Vec<(&'static str, String)>,
        calls: RefCell<Vec<String>>,
        /// GitHub refuses every end, as it refused both cancels of run 43095721996 (409)
        refuse: bool,
    }

    impl Forge {
        fn ask(&self, words: &[&str]) -> Result<String, String> {
            let said = words.join(" ");
            self.calls.borrow_mut().push(said.clone());
            if said.contains("status=in_progress") {
                return Ok(self.running.clone());
            }
            if said.contains("status=queued") {
                return Ok(self.queued.clone());
            }
            if words.contains(&"-X") {
                return match self.refuse {
                    true => Err("HTTP 409".into()),
                    false => Ok(String::new()),
                };
            }
            if let Some(path) = said.split("contents/").nth(1) {
                let path = path.split("?ref=").next().unwrap_or(path);
                return workflow(path).ok_or(format!("no stand-in for {said}"));
            }
            let job = self
                .jobs
                .iter()
                .find(|(id, _)| said.contains(&format!("runs/{id}/jobs")));
            job.map(|(_, j)| j.clone())
                .ok_or(format!("no stand-in for {said}"))
        }

        fn posts(&self) -> Vec<String> {
            let calls = self.calls.borrow();
            calls
                .iter()
                .filter(|c| c.contains(" -X "))
                .cloned()
                .collect()
        }
    }

    fn workflow(path: &str) -> Option<String> {
        match path {
            ".github/workflows/explainer-gate.yml" => Some(GATE.into()),
            ".github/workflows/dashboard.yml" => Some(MAC.into()),
            _ => None,
        }
    }

    fn run(f: &Forge, now: i64) -> (String, Vec<String>) {
        let (line, ended, _) = pass(&|w| f.ask(w), "nha-org/site_perso", now).expect("a pass");
        (line, ended)
    }

    /// Done-when 1 and run 43095022385's case (done-when 2): GitHub still holds the VPS gate in progress past its 6
    /// minutes plus 5, so it is cancelled and named; 5 minutes later, still held, the cancel is forced.
    #[test]
    fn a_vps_job_past_its_limit_plus_five_is_cancelled_then_forced() {
        let _budget = TestBudget::new();
        let f = Forge {
            running:
                "43095022385\t.github/workflows/explainer-gate.yml\tabc\t2026-11-11T20:16:58Z\n"
                    .into(),
            queued: String::new(),
            jobs: vec![(
                "43095022385",
                "gate\tin_progress\tsite-perso-vps\t2026-11-11T20:17:00Z\n".into(),
            )],
            calls: RefCell::default(),
            refuse: false,
        };
        let (line, ended) = run(&f, START + 10 * 60);
        assert!(
            ended.is_empty() && f.posts().is_empty(),
            "10 min in, under 6 + 5: {line}"
        );
        let (line, ended) = run(&f, START + 11 * 60);
        assert_eq!(
            f.posts(),
            ["api -X POST repos/nha-org/site_perso/actions/runs/43095022385/cancel"]
        );
        assert_eq!(
            ended,
            ["run 43095022385 (explainer-gate.yml gate) cancelled: 5 min past its 6-minute limit"]
        );
        assert!(
            line.starts_with("site-perso-vps: 1 running, 0 queued; run 43095022385"),
            "{line}"
        );
        run(&f, START + 16 * 60);
        assert_eq!(
            f.posts().last().map(String::as_str),
            Some("api -X POST repos/nha-org/site_perso/actions/runs/43095022385/force-cancel")
        );
    }

    /// Only the VPS label: a Mac job as old is left alone, and so is a VPS job inside its limit.
    #[test]
    fn a_job_off_the_vps_or_inside_its_limit_is_left() {
        let _budget = TestBudget::new();
        let f = Forge {
            running: "1\t.github/workflows/dashboard.yml\tabc\t2026-11-11T20:16:58Z\n\
                      2\t.github/workflows/explainer-gate.yml\tabc\t2026-11-11T21:00:00Z\n"
                .into(),
            queued: String::new(),
            jobs: vec![
                ("1", "check\tin_progress\tself-hosted,site-perso\t2026-11-11T20:17:00Z\n".into()),
                ("2", "gate\tin_progress\tsite-perso-vps\t2026-11-11T21:00:00Z\ngate\tcompleted\tsite-perso-vps\t2026-11-11T20:00:00Z\n".into()),
            ],
            calls: RefCell::default(),
            refuse: false,
        };
        let (line, ended) = run(&f, START + 48 * 60); // 21:05: the Mac job 48 min in, past 40 + 5
        assert!(ended.is_empty() && f.posts().is_empty(), "{line}");
        assert_eq!(
            line,
            "site-perso-vps: 2 running, 0 queued; nothing overdue, no run without a job"
        );
    }

    /// Run 43104132006's case (done-when 2): the « 63 minutes » /status read on the VPS runner was run 43095721996, queued
    /// with no job since 20:23. It is named as no hang of a running job, and nothing is asked of it, since GitHub refuses
    /// every end of it; a run queued with its job waiting, or younger than 15 minutes, or off the VPS, is not named.
    #[test]
    fn a_vps_run_queued_with_no_job_is_named_never_a_running_job_s_hang() {
        let _budget = TestBudget::new();
        let f = Forge {
            running: String::new(),
            queued:
                "43095721996\t.github/workflows/explainer-gate.yml\tabc\t2026-11-11T20:23:06Z\n\
                     7\t.github/workflows/explainer-gate.yml\tabc\t2026-11-11T20:23:06Z\n\
                     8\t.github/workflows/explainer-gate.yml\tabc\t2026-11-11T21:30:00Z\n\
                     9\t.github/workflows/dashboard.yml\tabc\t2026-11-11T20:23:06Z\n"
                    .into(),
            // run 9 has no job either, but its workflow runs on the Mac: not named
            jobs: vec![
                ("43095721996", "0\n".into()),
                ("7", "1\n".into()),
                ("9", "0\n".into()),
            ],
            calls: RefCell::default(),
            refuse: false,
        };
        let at = START + 76 * 60; // 21:33, when the orchestrator read « 63 minutes »
        let (line, ended) = run(&f, at);
        assert!(
            pass(&|w| f.ask(w), "nha-org/site_perso", at)
                .expect("a pass")
                .2,
            "a ghost holds no runner: OK"
        );
        assert!(f.posts().is_empty() && ended.is_empty(), "{line}");
        assert_eq!(
            line,
            "site-perso-vps: 0 running, 4 queued; run 43095721996 (explainer-gate.yml) queued 69 min with no job: GitHub \
             dropped it, it never starts and holds no runner, so no running job is hung for it; its PR's gate runs again on \
             its next push"
        );
    }

    /// Run 43104132006's hang, replayed (done-when 2): at 21:34:11 its gate had run 21 s on the VPS while run 43095721996 sat
    /// queued with no job since 20:23, which /status read as « 63 minutes on the VPS runner ». The cause was never the gate:
    /// the pass leaves the live gate running and names the jobless run as the queue's, so nothing cancels live work for it.
    #[test]
    fn run_43104132006_a_live_gate_beside_a_jobless_run_is_left_running() {
        let _budget = TestBudget::new();
        let f = Forge {
            running:
                "43104132006\t.github/workflows/explainer-gate.yml\tb4413a2\t2026-11-11T21:33:44Z\n"
                    .into(),
            queued:
                "43095721996\t.github/workflows/explainer-gate.yml\t8f26405\t2026-11-11T20:23:06Z\n"
                    .into(),
            jobs: vec![
                (
                    "43104132006",
                    "gate\tin_progress\tsite-perso-vps\t2026-11-11T21:33:50Z\n".into(),
                ),
                ("43095721996", "0\n".into()),
            ],
            calls: RefCell::default(),
            refuse: false,
        };
        let at = START + 77 * 60 + 11; // 21:34:11, the orchestrator's cancel
        let (line, ended, ok) = pass(&|w| f.ask(w), "nha-org/site_perso", at).expect("a pass");
        assert!(
            f.posts().is_empty() && ended.is_empty() && ok,
            "the live gate is never cancelled: {line}"
        );
        assert!(
            line.starts_with("site-perso-vps: 1 running, 1 queued; run 43095721996 (explainer-gate.yml) queued 71 min with no job"),
            "{line}"
        );
    }

    /// A refused cancel is named and the pass goes on to the next run.
    #[test]
    fn a_refused_cancel_is_named_and_the_pass_goes_on() {
        let _budget = TestBudget::new();
        let gate = "gate\tin_progress\tsite-perso-vps\t2026-11-11T20:17:00Z\n";
        let f = Forge {
            running: "5\t.github/workflows/explainer-gate.yml\tabc\t2026-11-11T20:16:58Z\n\
                      6\t.github/workflows/explainer-gate.yml\tabc\t2026-11-11T20:16:58Z\n"
                .into(),
            queued: String::new(),
            jobs: vec![("5", gate.into()), ("6", gate.into())],
            calls: RefCell::default(),
            refuse: true,
        };
        let ok = |at| {
            pass(&|w| f.ask(w), "nha-org/site_perso", at)
                .expect("a pass")
                .2
        };
        assert!(!ok(START + 12 * 60), "a refused cancel fails the pass");
        let (_, ended) = run(&f, START + 12 * 60);
        assert_eq!(
            ended,
            [
                "run 5: cancel refused: HTTP 409",
                "run 6: cancel refused: HTTP 409"
            ]
        );
    }

    /// A run whose jobs or workflow do not read is named unread, and the pass still ends the overdue run after it; the limit
    /// is read from the run's own commit.
    #[test]
    fn an_unread_run_is_named_and_never_hides_an_overdue_one() {
        let _budget = TestBudget::new();
        let gate = "gate\tin_progress\tsite-perso-vps\t2026-11-11T20:17:00Z\n";
        let f = Forge {
            running: "3\t.github/workflows/explainer-gate.yml\tabc\t2026-11-11T20:16:58Z\n\
                      4\t.github/workflows/gone.yml\tabc\t2026-11-11T20:16:58Z\n\
                      5\t.github/workflows/explainer-gate.yml\tdef\t2026-11-11T20:16:58Z\n"
                .into(),
            queued: String::new(),
            jobs: vec![("4", gate.into()), ("5", gate.into())],
            calls: RefCell::default(),
            refuse: false,
        };
        let (line, ended) = run(&f, START + 12 * 60);
        assert_eq!(
            ended,
            ["run 5 (explainer-gate.yml gate) cancelled: 6 min past its 6-minute limit"]
        );
        assert!(
            !pass(&|w| f.ask(w), "nha-org/site_perso", START + 12 * 60)
                .expect("a pass")
                .2
        );
        assert!(
            line.contains("run 3: jobs unread: ") && line.contains("run 4: gone.yml unread: "),
            "{line}"
        );
        let calls = f.calls.borrow();
        assert!(
            calls
                .iter()
                .any(|c| c.contains("contents/.github/workflows/explainer-gate.yml?ref=def")),
            "{calls:?}"
        );
    }

    /// The limit is the workflow's own number; an expression or none falls back to 45.
    #[test]
    fn the_limit_is_the_workflow_s_timeout_minutes() {
        let _budget = TestBudget::new();
        assert_eq!(limit_of(GATE), 6);
        assert_eq!(
            limit_of("    timeout-minutes: ${{ x && 45 || 25 }}\n"),
            DEFAULT_LIMIT_MIN
        );
        assert_eq!(limit_of("a: 1\n"), DEFAULT_LIMIT_MIN);
        assert_eq!(limit_of("    timeout-minutes: 6 # the gate\n"), 6);
        assert_eq!(limit_of("    timeout-minutes: '6'\n"), 6);
        // dashboard.yml's shape: its define-ci job's literal 6 beside the VPS route's expression, never read as 6
        let dashboard = "  define-ci:\n    timeout-minutes: 6\n  test:\n    timeout-minutes: ${{ contains(x, 'site-perso-vps') && 45 || 25 }}\n";
        assert_eq!(limit_of(dashboard), DEFAULT_LIMIT_MIN);
    }
}
