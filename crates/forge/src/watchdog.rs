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
use crate::queue;
use crate::runs::unix;

/// The label of the VPS runners (bin/ci-runner vps).
pub const LABEL: &str = "site-perso-vps";
/// Minutes past a job's own limit before it is cancelled, and again before the cancel is forced.
pub const GRACE_MIN: i64 = 5;
/// Minutes a run waits with no job before it counts as dropped: GitHub creates a run's jobs within seconds.
pub const GHOST_MIN: i64 = queue::ENDED_MIN;
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

/// A pass's line for /watchers, each run it ended, whether it is OK, and the CI queue row: OK, and what it says.
pub type Pass = (String, Vec<String>, bool, (bool, String));

/// One pass at `now` (Unix seconds) on `slug`: the line for /watchers, one line for each run it ended or was refused, and
/// whether the pass is OK: false once GitHub refused an end or a running run did not read, since either may hide a hold. Each
/// run's workflow is read at its own head commit, so a branch that changes its limit or its runner is judged by its own file.
/// A run whose jobs or workflow do not read is named « unread » and the pass goes on.
/// Last, the CI queue row (crate::queue): each run's state from the jobs this pass read, with no call of its own.
pub fn pass(
    ask: &dyn Fn(&[&str]) -> Result<String, String>,
    slug: &str,
    now: i64,
) -> Result<Pass, String> {
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
            // 100 a page, GitHub's most: its default of 30 could leave a matrix run's waiting job unread
            &format!("repos/{slug}/actions/runs/{id}/jobs?per_page=100"),
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
    // a run pending behind its concurrency group has no job yet: it waits from its creation, as the old row read it
    let pending = runs("pending")?;
    let (mut ended, mut notes, mut ok) = (vec![], vec![], true);
    // the queue row's runs: each waiting one's workflow and since when, each dropped one named
    let (mut waiting, mut dropped) = (vec![], vec![]);
    let jq = ".jobs[] | [.name, .status, (.labels | join(\",\")), (.started_at // \"\"), .created_at, (.completed_at // \"\")] | @tsv";
    for (id, path, sha, created) in &running {
        let rows = match jobs(id, jq) {
            Ok(rows) => rows,
            Err(e) => {
                notes.push(format!("run {id}: jobs unread: {e}"));
                ok = false;
                continue;
            }
        };
        place(
            &mut waiting,
            &mut dropped,
            id,
            path,
            *created,
            Some(&rows),
            now,
        );
        // the oldest job in progress on the label: the only one read further
        let Some((job, started)) = rows
            .lines()
            .filter_map(|row| match row.split('\t').collect::<Vec<_>>()[..] {
                [job, "in_progress", labels, started, ..]
                    if labels.split(',').any(|l| l == LABEL) =>
                {
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
        // a younger run is read as waiting since its creation: GitHub creates a run's jobs within seconds, and its wait is
        // under the queue's limit anyway
        if waited < GHOST_MIN {
            place(&mut waiting, &mut dropped, id, path, *created, None, now);
            continue;
        }
        // a queued run holds no runner, so an unread one is named but never fails the pass: it may be the Mac's
        let rows = match jobs(id, jq) {
            Ok(rows) => rows,
            Err(e) => {
                notes.push(format!("run {id}: jobs unread: {e}"));
                place(&mut waiting, &mut dropped, id, path, *created, None, now);
                continue;
            }
        };
        place(
            &mut waiting,
            &mut dropped,
            id,
            path,
            *created,
            Some(&rows),
            now,
        );
        // a run with a job is no ghost
        if !rows.trim().is_empty() {
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
    for (id, path, _, created) in &pending {
        place(&mut waiting, &mut dropped, id, path, *created, None, now);
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
    Ok((line, ended, ok, queue::row(&waiting, &dropped, now)))
}

/// Where a run stands in the queue row's lists by `rows`, the pass's job lines (None when not read), its workflow named by
/// its file's stem: the re-probe's runs never count. A running run whose jobs did not read is left out by its caller: it holds
/// a runner, as the old row read it.
fn place(
    waiting: &mut Vec<(String, i64)>,
    dropped: &mut Vec<String>,
    id: &str,
    path: &str,
    created: i64,
    rows: Option<&str>,
    now: i64,
) {
    let file = path.rsplit('/').next().unwrap_or(path);
    let workflow = file.trim_end_matches(".yml").trim_end_matches(".yaml");
    if workflow == queue::PROBE {
        return;
    }
    let jobs: Option<Vec<queue::Job>> = rows.map(|rows| {
        (rows.lines())
            .filter_map(|row| {
                let cols: Vec<&str> = row.split('\t').collect();
                let at = |i: usize| cols.get(i).and_then(|c| unix(c));
                Some((*cols.get(1)?, at(4), at(5)))
            })
            .collect()
    });
    match queue::state(created, jobs.as_deref(), now) {
        queue::State::Waiting(since) => waiting.push((workflow.to_string(), since)),
        queue::State::Running => {}
        queue::State::Dropped => dropped.push(format!("run {id} ({workflow})")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;
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
        pending: String,
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
            if said.contains("status=pending") {
                return Ok(self.pending.clone());
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
        let (line, ended, _, _) = pass(&|w| f.ask(w), "nha-org/site_perso", now).expect("a pass");
        (line, ended)
    }

    /// Done-when 1 and run 43095022385's case (done-when 2): GitHub still holds the VPS gate in progress past its 6
    /// minutes plus 5, so it is cancelled and named; 5 minutes later, still held, the cancel is forced.
    #[dash_test]
    fn a_vps_job_past_its_limit_plus_five_is_cancelled_then_forced() {
        let f = Forge {
            running:
                "43095022385\t.github/workflows/explainer-gate.yml\tabc\t2026-11-11T20:16:58Z\n"
                    .into(),
            queued: String::new(),
            pending: String::new(),
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
    #[dash_test]
    fn a_job_off_the_vps_or_inside_its_limit_is_left() {
        let f = Forge {
            running: "1\t.github/workflows/dashboard.yml\tabc\t2026-11-11T20:16:58Z\n\
                      2\t.github/workflows/explainer-gate.yml\tabc\t2026-11-11T21:00:00Z\n"
                .into(),
            queued: String::new(),
            pending: String::new(),
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
    #[dash_test]
    fn a_vps_run_queued_with_no_job_is_named_never_a_running_job_s_hang() {
        let f = Forge {
            running: String::new(),
            queued:
                "43095721996\t.github/workflows/explainer-gate.yml\tabc\t2026-11-11T20:23:06Z\n\
                     7\t.github/workflows/explainer-gate.yml\tabc\t2026-11-11T20:23:06Z\n\
                     8\t.github/workflows/explainer-gate.yml\tabc\t2026-11-11T21:30:00Z\n\
                     9\t.github/workflows/dashboard.yml\tabc\t2026-11-11T20:23:06Z\n"
                    .into(),
            // run 9 has no job either, but its workflow runs on the Mac: not named
            pending: String::new(),
            jobs: vec![
                ("43095721996", String::new()),
                (
                    "7",
                    "gate\tqueued\tsite-perso-vps\t\t2026-11-11T20:23:08Z\n".into(),
                ),
                ("9", String::new()),
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
    #[dash_test]
    fn run_43104132006_a_live_gate_beside_a_jobless_run_is_left_running() {
        let f = Forge {
            running:
                "43104132006\t.github/workflows/explainer-gate.yml\tb4413a2\t2026-11-11T21:33:44Z\n"
                    .into(),
            queued:
                "43095721996\t.github/workflows/explainer-gate.yml\t8f26405\t2026-11-11T20:23:06Z\n"
                    .into(),
            pending: String::new(),
            jobs: vec![
                (
                    "43104132006",
                    "gate\tin_progress\tsite-perso-vps\t2026-11-11T21:33:50Z\n".into(),
                ),
                ("43095721996", String::new()),
            ],
            calls: RefCell::default(),
            refuse: false,
        };
        let at = START + 77 * 60 + 11; // 21:34:11, the orchestrator's cancel
        let (line, ended, ok, _) = pass(&|w| f.ask(w), "nha-org/site_perso", at).expect("a pass");
        assert!(
            f.posts().is_empty() && ended.is_empty() && ok,
            "the live gate is never cancelled: {line}"
        );
        assert!(
            line.starts_with("site-perso-vps: 1 running, 1 queued; run 43095721996 (explainer-gate.yml) queued 71 min with no job"),
            "{line}"
        );
    }

    /// Done-when 2 of rust-ci-queue-port, through the pass: the queue row reads each run's state from its jobs. Run
    /// 43095721996, listed queued with no job since 20:23, and a run listed in progress whose jobs all completed are left out
    /// and named; a dashboard run whose define-ci ended and whose routed job waits counts from that job's creation; the
    /// re-probe never counts; a run pending behind its concurrency group waits from its creation. The old row read 43095721996 as « 63 minutes », red.
    #[dash_test]
    fn the_queue_row_drops_a_run_whose_jobs_ended_or_never_came() {
        let f = Forge {
            running: "11\t.github/workflows/explainer-gate.yml\tabc\t2026-11-11T21:00:00Z\n\
                      12\t.github/workflows/dashboard.yml\tabc\t2026-11-11T21:20:00Z\n"
                .into(),
            queued:
                "43095721996\t.github/workflows/explainer-gate.yml\t8f26405\t2026-11-11T20:23:06Z\n\
                 13\t.github/workflows/ci-probe.yml\tabc\t2026-11-11T20:23:06Z\n"
                    .into(),
            pending: "14\t.github/workflows/check.yml\tabc\t2026-11-11T21:25:00Z\n".into(),
            jobs: vec![
                (
                    "11",
                    "gate\tcompleted\tsite-perso-vps\t2026-11-11T21:00:05Z\t2026-11-11T21:00:01Z\t2026-11-11T21:05:00Z\n".into(),
                ),
                (
                    "12",
                    "define-ci\tcompleted\tubuntu-latest\t2026-11-11T21:20:05Z\t2026-11-11T21:20:01Z\t2026-11-11T21:20:50Z\n\
                     check\tqueued\tself-hosted,site-perso\t\t2026-11-11T21:21:00Z\n"
                        .into(),
                ),
                ("43095721996", String::new()),
                ("13", String::new()),
            ],
            calls: RefCell::default(),
            refuse: false,
        };
        let at = START + 76 * 60; // 21:33
        let (_, _, _, (ok, said)) = pass(&|w| f.ask(w), "nha-org/site_perso", at).expect("a pass");
        assert!(ok, "12 minutes of wait: {said}");
        assert_eq!(
            said,
            "2 queued, oldest waiting 12 min: dashboard 1 (oldest 12 min); check 1 (oldest 8 min); left out, listed by GitHub though no job of it \
             waits or runs: run 11 (explainer-gate), run 43095721996 (explainer-gate)"
        );
    }

    /// A refused cancel is named and the pass goes on to the next run.
    #[dash_test]
    fn a_refused_cancel_is_named_and_the_pass_goes_on() {
        let gate = "gate\tin_progress\tsite-perso-vps\t2026-11-11T20:17:00Z\n";
        let f = Forge {
            running: "5\t.github/workflows/explainer-gate.yml\tabc\t2026-11-11T20:16:58Z\n\
                      6\t.github/workflows/explainer-gate.yml\tabc\t2026-11-11T20:16:58Z\n"
                .into(),
            queued: String::new(),
            pending: String::new(),
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
    #[dash_test]
    fn an_unread_run_is_named_and_never_hides_an_overdue_one() {
        let gate = "gate\tin_progress\tsite-perso-vps\t2026-11-11T20:17:00Z\n";
        let f = Forge {
            running: "3\t.github/workflows/explainer-gate.yml\tabc\t2026-11-11T20:16:58Z\n\
                      4\t.github/workflows/gone.yml\tabc\t2026-11-11T20:16:58Z\n\
                      5\t.github/workflows/explainer-gate.yml\tdef\t2026-11-11T20:16:58Z\n"
                .into(),
            queued: String::new(),
            pending: String::new(),
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
    #[dash_test]
    fn the_limit_is_the_workflow_s_timeout_minutes() {
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
