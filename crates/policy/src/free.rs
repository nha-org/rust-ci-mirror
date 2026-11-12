//! RFC 20e's free models, the picker's half (the kinds and their labels are core's `FreeKind`): which model takes which kind of task, one session per tool, and the stall bound
//! that hands a task back to a seat (task rust-orch-pick-dynamic, done-when 4). Launching them is rust-free-runner's (the
//! orchestrator's answer in this task's question dialog, 2026-11-10: « decisions here with tests (kind to model, one session
//! per tool, the stall rule). Moved to rust-free-runner. »).
use dash_core::tasks::{
    FreeKind, Pick, Plan, Policy, Queue, Queued, Snapshot, Standing, Step, Verdict, plan_at,
};

/// RFC 20e's table: each kind's tool and model. All through OpenCode in its own sandbox today; Gemini CLI and Freebuff are tools
/// of their own, each one session at a time, once a kind goes to them.
pub const TABLE: [(FreeKind, &str, &str); 4] = [
    (FreeKind::Planning, "opencode", "big-pickle"),
    (FreeKind::Build, "opencode", "beam-free"),
    (FreeKind::Script, "opencode", "glm-5.3-flash-free"),
    (FreeKind::BugHunt, "opencode", "deepseek-v4.1-flash-free"),
];

/// Its tool and model.
pub fn model(kind: FreeKind) -> (&'static str, &'static str) {
    let row = TABLE
        .iter()
        .find(|row| row.0 == kind)
        .expect("every kind has a row");
    (row.1, row.2)
}

/// A free run's time with no token moved before it is stopped and a seat takes its task (RFC 20e: Beam read and searched an
/// hour on rust-gate-says-fix, 48K tokens, no edit, 2026-11-10).
pub const STALL_MS: i64 = 10 * 60 * 1000;

/// A free session the runner started: `last_token_ms` when its token count last moved (Unix milliseconds), `ended` once its
/// process is gone. Until then it holds its tool, stalled or not: only a stop that took frees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub id: String,
    pub tool: String,
    pub last_token_ms: i64,
    pub ended: bool,
}

/// A free run to start: the task, its tool and model.
#[derive(Clone, Debug, PartialEq)]
pub struct Start {
    pub id: String,
    pub tool: &'static str,
    pub model: &'static str,
}

/// Whether `run` moved no token for `STALL_MS`.
pub fn stalled(run: &Run, now_ms: i64) -> bool {
    now_ms - run.last_token_ms >= STALL_MS
}

/// Before the seats' plan: a live free run holds its task from them; a stalled one is to be stopped, its task left to a seat,
/// whether or not the list still offers it. Returns those to stop, each with why.
pub fn hold(queue: &mut Queue, runs: &[Run], now_ms: i64) -> Vec<(String, String)> {
    let mut stop = vec![];
    for run in runs.iter().filter(|r| !r.ended) {
        if stalled(run, now_ms) {
            let why = "no token moved in 10 minutes: a seat takes it";
            stop.push((run.id.clone(), why.into()));
        } else if let Some(task) = queue.tasks.iter_mut().find(|t| t.id == run.id)
            && task.held.is_none()
        {
            task.held = Some(format!("a free session runs it on {}", run.tool));
        }
    }
    stop
}

/// The first task a free tool may start now, after the seats' `plan`: a kind of the table, never the seats' pick, never one a
/// free run already had (a stalled one is a seat's from then on), its tool's one session free (@nha, 2026-11-10: « I suggest for
/// these free tools to use them SEQUENTIALLY because their limits run out with parallel sessions and work needs to be redone »).
/// Every hold of the seats' plan stands but theirs: its waits, his drag, a layer at work, memory short for a build.
pub fn start(
    policy: &dyn Policy,
    seats: &Plan,
    queue: &Queue,
    rows: &[Standing],
    runs: &[Run],
    free_percent: Option<u8>,
    load: Option<f64>,
) -> Option<Start> {
    let picked = match &seats.next {
        Pick::Start { id, .. } | Pick::Adopt(id) => Some(id.as_str()),
        Pick::Wait(_) => None,
    };
    // a stalled run holds its tool until its stop took: two sessions at once hit the free cap together
    let busy = |tool: &str| runs.iter().any(|r| r.tool == tool && !r.ended);
    let open = |t: &&Queued| {
        let tool = t.free.map(|k| model(k).0);
        tool.is_some_and(|tool| !busy(tool))
            && picked != Some(t.id.as_str())
            && !runs.iter().any(|r| r.id == t.id)
    };
    let tasks = queue.tasks.iter().filter(open).cloned().collect();
    let free = Queue {
        seats: 1,
        seats_why: String::new(),
        tasks,
    };
    // no seat spent, but the load stops a free run as it stops a seat's (task rust-seats-follow-room)
    let first = plan_at(&First(policy), free, rows, 0, vec![], free_percent, load, 0);
    let Pick::Start { id, .. } = first.next else {
        return None;
    };
    let kind = queue.tasks.iter().find(|t| t.id == id)?.free?;
    let (tool, model) = model(kind);
    Some(Start { id, tool, model })
}

/// `policy`'s holds, with no seat or account to count: a free tool spends neither.
struct First<'a>(&'a dyn Policy);

impl Policy for First<'_> {
    fn pick(&self, snapshot: &Snapshot) -> Pick {
        let free = snapshot.free_percent;
        let first = snapshot
            .queue
            .tasks
            .iter()
            .find(|t| self.0.holds(t, free).is_none());
        first.map_or(Pick::Wait("nothing free".into()), |t| Pick::Start {
            id: t.id.clone(),
            account: None,
            pool: None,
        })
    }
    fn after(&self, verdict: &Verdict, nudges: u32, past_limit: bool) -> Step {
        self.0.after(verdict, nudges, past_limit)
    }
    fn retry(&self, attempts: u32) -> bool {
        self.0.retry(attempts)
    }
    fn holds(&self, task: &Queued, free_percent: Option<u8>) -> Option<String> {
        self.0.holds(task, free_percent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_core::tasks::plan;
    use dash_meter::memory::TestBudget;

    /// Holds nothing back and picks the first offered, as `Defaults` does with room: what is tested here is the free lane alone.
    struct Open;

    impl Policy for Open {
        fn pick(&self, snapshot: &Snapshot) -> Pick {
            let first = snapshot.queue.tasks.first();
            first.map_or(Pick::Wait("nothing".into()), |t| Pick::Start {
                id: t.id.clone(),
                account: None,
                pool: None,
            })
        }
        fn after(&self, _: &Verdict, _: u32, _: bool) -> Step {
            Step::Keep
        }
        fn retry(&self, _: u32) -> bool {
            false
        }
    }

    fn task(id: &str, labels: &[&str]) -> Queued {
        let labels: Vec<String> = labels.iter().map(|l| l.to_string()).collect();
        Queued {
            id: id.into(),
            free: FreeKind::of(&labels),
            ..Queued::default()
        }
    }

    fn queue(tasks: Vec<Queued>) -> Queue {
        Queue {
            seats: 5,
            seats_why: String::new(),
            tasks,
        }
    }

    fn run(id: &str, last_token_ms: i64) -> Run {
        let tool = "opencode".into();
        Run {
            id: id.into(),
            tool,
            last_token_ms,
            ended: false,
        }
    }

    /// The free lane's pick beside the seats' plan of the same queue.
    fn both(q: &Queue, runs: &[Run], now: i64) -> (Pick, Option<Start>) {
        let mut seats = q.clone();
        hold(&mut seats, runs, now);
        let plan = plan(&Open, seats.clone(), &[], 0, vec![], None);
        let free = start(&Open, &plan, &seats, &[], runs, None, None);
        (plan.next, free)
    }

    /// Done-when 4, case 1: each kind of work goes to RFC 20e's model for it; a task with no such label stays the seats'.
    #[test]
    fn each_kind_of_work_goes_to_the_model_rfc_20e_names_for_it() {
        let _budget = TestBudget::new();
        let model = |labels: &[&str]| task("t", labels).free.map(super::model);
        assert_eq!(model(&["Docs"]), Some(("opencode", "big-pickle")));
        assert_eq!(model(&["research"]), Some(("opencode", "big-pickle")));
        assert_eq!(model(&["Free"]), Some(("opencode", "beam-free")));
        assert_eq!(model(&["Script"]), Some(("opencode", "glm-5.3-flash-free")));
        assert_eq!(
            model(&["Bug"]),
            Some(("opencode", "deepseek-v4.1-flash-free"))
        );
        assert_eq!(
            model(&["Feature"]),
            None,
            "a build goes free only when labelled so"
        );
        for (kind, _, _) in TABLE {
            assert_eq!(
                FreeKind::of(&[kind.label().to_string()]),
                Some(kind),
                "{kind:?} reads back"
            );
        }
        // the free lane starts it on that model, never the seats' own pick
        let q = queue(vec![task("build", &[]), task("hunt", &["Bug"])]);
        let start = Start {
            id: "hunt".into(),
            tool: "opencode",
            model: "deepseek-v4.1-flash-free",
        };
        let seat = Pick::Start {
            id: "build".into(),
            account: None,
            pool: None,
        };
        assert_eq!(both(&q, &[], 0), (seat, Some(start)));
        let alone = queue(vec![task("plan", &["Docs"])]);
        assert_eq!(
            both(&alone, &[], 0).1,
            None,
            "the seats take it: one task never runs twice"
        );
    }

    /// Done-when 4, case 2: one session per tool. While a free run is live on OpenCode no other free task starts there, and its
    /// own task is held from the seats; a task a free run already had never goes free again.
    #[test]
    fn a_tool_runs_one_free_session_at_a_time() {
        let _budget = TestBudget::new();
        let q = queue(vec![
            task("build", &[]),
            task("triage", &["Docs"]),
            task("fix", &["Script"]),
        ]);
        let live = [run("triage", 0)];
        let (seat, free) = both(&q, &live, STALL_MS - 1);
        assert_eq!(
            seat,
            Pick::Start {
                id: "build".into(),
                account: None,
                pool: None,
            }
        );
        assert_eq!(free, None, "fix waits for OpenCode's one session");
        let mut held = q.clone();
        assert!(hold(&mut held, &live, STALL_MS - 1).is_empty());
        assert_eq!(
            held.tasks[1].held.as_deref(),
            Some("a free session runs it on opencode")
        );
        // stalled and not stopped yet, it still holds OpenCode
        assert_eq!(
            both(&q, &live, STALL_MS).1,
            None,
            "a stalled run holds its tool until it ends"
        );
        // once that session ended, the next free task starts; the one it ran is never free again
        let done = [Run {
            ended: true,
            ..run("triage", 0)
        }];
        let start = Start {
            id: "fix".into(),
            tool: "opencode",
            model: "glm-5.3-flash-free",
        };
        assert_eq!(both(&q, &done, STALL_MS - 1).1, Some(start));
    }

    /// Done-when 4, case 3: 10 minutes with no token moved stops the free run, and a seat takes its task.
    #[test]
    fn a_free_run_with_no_token_for_ten_minutes_stops_and_a_seat_takes_its_task() {
        let _budget = TestBudget::new();
        let q = queue(vec![task("triage", &["Docs"])]);
        let runs = [run("triage", 1_000)];
        let mut seats = q.clone();
        assert!(
            hold(&mut seats, &runs, 1_000 + STALL_MS - 1).is_empty(),
            "nine minutes and more: still live"
        );
        assert!(
            matches!(both(&q, &runs, 1_000 + STALL_MS - 1).0, Pick::Wait(_)),
            "held from the seats meanwhile"
        );
        // a stalled run is stopped even once the list no longer offers its task
        let gone = hold(&mut queue(vec![]), &runs, 1_000 + STALL_MS);
        assert_eq!(gone.len(), 1, "{gone:?}");
        let mut seats = q.clone();
        let stop = hold(&mut seats, &runs, 1_000 + STALL_MS);
        assert_eq!(
            stop,
            [(
                "triage".to_string(),
                "no token moved in 10 minutes: a seat takes it".to_string()
            )]
        );
        let (seat, free) = both(&q, &runs, 1_000 + STALL_MS);
        assert_eq!(
            seat,
            Pick::Start {
                id: "triage".into(),
                account: None,
                pool: None,
            }
        );
        assert_eq!(free, None, "never free again");
    }
}

#[cfg(test)]
mod load {
    use super::*;
    use crate::Defaults;
    use dash_core::tasks::plan;
    use dash_meter::memory::TestBudget;

    /// Task rust-seats-follow-room's /code-review: past 3 per core no free run starts either, though it spends no seat.
    #[test]
    fn past_the_load_line_no_free_run_starts() {
        let _budget = TestBudget::new();
        let hunt = Queued {
            id: "hunt".into(),
            free: FreeKind::of(&["Bug".to_string()]),
            ..Queued::default()
        };
        let q = Queue {
            seats: 5,
            seats_why: "day 5".into(),
            tasks: vec![hunt],
        };
        let seats = plan(&Defaults, q.clone(), &[], 5, vec![], None);
        let free = |load| start(&Defaults, &seats, &q, &[], &[], None, load).map(|s| s.id);
        assert_eq!(
            free(Some(1.0)),
            Some("hunt".into()),
            "under the line it starts"
        );
        assert_eq!(free(Some(3.5)), None, "past it, nothing starts");
    }
}
