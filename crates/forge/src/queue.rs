//! The CI queue row (task rust-ci-queue-port, which ports Dashboard.CiQueue's /status row): the runs waiting for a runner and
//! the oldest wait per workflow, red past [`LIMIT_MIN`]. A run's place in the queue comes from its jobs, never from the status
//! GitHub lists it under: run 43095721996 stayed listed « queued » from 2026-11-11 20:23 with no job at all (GitHub dropped
//! it when the VPS runner's broker poll stalled), and the old row read it as a 10-hour wait, an outage that woke the
//! orchestrator each hour. The watchdog's pass (crate::watchdog) reads the jobs and hands each run's here.

/// Minutes of wait past which the queue is red (Dashboard.CiQueue's `@limit`).
pub const LIMIT_MIN: i64 = 30;
/// The re-probe's workflow (Dashboard.CiWatch's `@probe`): it waits by design on a provider still refusing, never a queue.
pub const PROBE: &str = "ci-probe";

/// Where a run GitHub lists as queued or in progress stands, by its jobs.
#[derive(Debug, PartialEq)]
pub enum State {
    /// A job of it waits for a runner since this Unix second, and none runs.
    Waiting(i64),
    /// A job of it runs (it holds a runner, as the old row read every run in progress), or its last job ended under
    /// [`ENDED_MIN`] ago: GitHub has yet to close it or to create its next job.
    Running,
    /// It has no job, or its jobs all ended [`ENDED_MIN`] ago or more: GitHub's listing is stale, the run never starts again.
    Dropped,
}

/// Minutes after a run's last job ended before a run still listed is dropped, so the seconds between one job's end and the
/// next one's creation never name a run; also the watchdog's ghost bound (crate::watchdog::GHOST_MIN): GitHub creates a run's
/// jobs within seconds.
pub const ENDED_MIN: i64 = 15;

/// One job, as the pass reads it: its status, when it was created and when it ended (Unix seconds, none when unread or not
/// ended).
pub type Job<'a> = (&'a str, Option<i64>, Option<i64>);

/// A run created at `created`, by its `jobs` at `now`: None when they were not read (a queued run younger than the watchdog's
/// ghost bound, a pending one, or a read that failed), which counts as waiting since its creation, the old row's reading, so an
/// unread run is never hidden.
pub fn state(created: i64, jobs: Option<&[Job]>, now: i64) -> State {
    let Some(jobs) = jobs else {
        return State::Waiting(created);
    };
    if jobs.iter().any(|(status, _, _)| *status == "in_progress") {
        return State::Running;
    }
    // queued, waiting, pending or requested: each holds the run in the queue, from its own creation
    let waiting = jobs.iter().filter(|(status, _, _)| *status != "completed");
    if let Some(since) = waiting.map(|(_, at, _)| at.unwrap_or(created)).min() {
        return State::Waiting(since);
    }
    match jobs.iter().filter_map(|(_, _, ended)| *ended).max() {
        Some(last) if now - last < ENDED_MIN * 60 => State::Running,
        _ => State::Dropped,
    }
}

/// The row at `now`: whether it is OK and what it says, from each waiting run's workflow and since when, and each run dropped
/// (« run <id> (<workflow>) »), named so a reader sees why a listed run does not count.
pub fn row(waiting: &[(String, i64)], dropped: &[String], now: i64) -> (bool, String) {
    let mut per: Vec<(&str, usize, i64)> = vec![];
    for (workflow, since) in waiting {
        let wait = (now - since).max(0);
        match per.iter_mut().find(|(w, _, _)| w == workflow) {
            Some((_, n, oldest)) => {
                *n += 1;
                *oldest = (*oldest).max(wait);
            }
            None => per.push((workflow, 1, wait)),
        }
    }
    per.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(b.0)));
    let (ok, mut said) = match per.first() {
        None => (true, "no run is queued".to_string()),
        Some((_, _, oldest)) => {
            let each = per
                .iter()
                .map(|(w, n, o)| format!("{w} {n} (oldest {} min)", o / 60))
                .collect::<Vec<_>>()
                .join("; ");
            (
                *oldest <= LIMIT_MIN * 60,
                format!(
                    "{} queued, oldest waiting {} min: {each}",
                    waiting.len(),
                    oldest / 60
                ),
            )
        }
    };
    if !dropped.is_empty() {
        said += &format!(
            "; left out, listed by GitHub though no job of it waits or runs: {}",
            dropped.join(", ")
        );
    }
    (ok, said)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// Done-when 2: a run's state comes from its jobs. Run 43095721996's shape, listed queued with no job, and a run listed
    /// in progress whose jobs all completed are dropped; a queued job waits from its own creation, a running one holds none.
    #[dash_test]
    fn a_run_s_state_comes_from_its_jobs() {
        let now = 10_000;
        assert_eq!(state(100, Some(&[]), now), State::Dropped, "no job at all");
        let ended = [
            ("completed", Some(110), Some(150)),
            ("completed", Some(120), Some(200)),
        ];
        assert_eq!(
            state(100, Some(&ended), now),
            State::Dropped,
            "every job ended long ago"
        );
        assert_eq!(
            state(100, Some(&ended), 200 + 14 * 60),
            State::Running,
            "its last job ended 14 min ago: GitHub may yet close it or create its next job"
        );
        let routed = [
            ("completed", Some(110), Some(120)),
            ("queued", Some(130), None),
        ];
        assert_eq!(
            state(100, Some(&routed), now),
            State::Waiting(130),
            "define-ci ended, its routed job waits: in the queue though GitHub lists the run in progress"
        );
        let running = [
            ("in_progress", Some(110), None),
            ("pending", Some(105), None),
        ];
        assert_eq!(
            state(100, Some(&running), now),
            State::Running,
            "a job runs: the run holds a runner, its job behind `needs` waits on it, not on the queue"
        );
        assert_eq!(
            state(100, Some(&[("waiting", None, None)]), now),
            State::Waiting(100)
        );
        assert_eq!(
            state(100, None, now),
            State::Waiting(100),
            "unread: never hidden"
        );
    }

    /// Done-when 1's text, as the old row wrote it: per workflow, the longest wait first, red past 30 minutes; a dropped run
    /// named, never counted.
    #[dash_test]
    fn the_row_says_each_workflow_s_oldest_wait() {
        let now = 10_000;
        assert_eq!(row(&[], &[], now), (true, "no run is queued".into()));
        let waiting = [
            ("dashboard".to_string(), now - 5 * 60),
            ("explainer-gate".to_string(), now - 31 * 60),
            ("dashboard".to_string(), now - 12 * 60),
        ];
        let dropped = ["run 43095721996 (explainer-gate)".to_string()];
        assert_eq!(
            row(&waiting, &dropped, now),
            (
                false,
                "3 queued, oldest waiting 31 min: explainer-gate 1 (oldest 31 min); dashboard 2 (oldest 12 min); left out, \
                 listed by GitHub though no job of it waits or runs: run 43095721996 (explainer-gate)"
                    .into()
            )
        );
        assert!(row(&waiting[..1], &[], now).0, "5 min is under the limit");
    }
}
