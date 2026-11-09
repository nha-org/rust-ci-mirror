//! Core's Policy, the default one (section 20b): his seats, his spend order, and what follows a verdict, as the older app's
//! PM check does it. Pure, so a stand-in is only ever needed to test another policy.
use dash_core::tasks::{Pick, Policy, Snapshot, Step, Verdict};

/// A PM check's nudges before it gives up (`PM.max_nudges`), and the same for tries at a run.
const NUDGES: u32 = 3;
const ATTEMPTS: u32 = 3;

pub struct Defaults;

impl Policy for Defaults {
    fn pick(&self, snapshot: &Snapshot) -> Pick {
        let Some(task) = snapshot.queue.tasks.first() else {
            return Pick::Wait("nothing to pick".into());
        };
        let (open, seats) = (snapshot.sessions, snapshot.queue.seats);
        if open >= seats {
            return Pick::Wait(format!(
                "{} waits: {open} of {seats} sessions open",
                task.id
            ));
        }
        // Spend order: the account whose week resets first, to 100% of its 5 hours and its week, then the next
        // (@nha, 2026-11-01: « use 100% of one sub that expires first before moving to the next »).
        // A key's row is spent by a headless run alone, never a pane's (rust-key-agent's question dialog, 2026-11-08).
        let accounts: Vec<_> = snapshot.limits.iter().filter(|l| !l.key).collect();
        let mut room: Vec<_> = accounts.iter().filter(|l| l.room()).collect();
        room.sort_by_key(|l| l.week_resets);
        if room.is_empty() && !accounts.is_empty() {
            return Pick::Wait(format!("{} waits: every account is at its limit", task.id));
        }
        let account = room.first().map(|l| l.account.clone());
        let id = task.id.clone();
        Pick::Start { id, account }
    }

    fn after(&self, verdict: &Verdict, nudges: u32, past_limit: bool) -> Step {
        match verdict {
            Verdict::Met => Step::Land,
            // Working: a nudge would land mid-turn, and its next stop is judged anyway.
            Verdict::NotMet(_) if past_limit => Step::Keep,
            Verdict::NotMet(why) if nudges >= NUDGES => {
                Step::GiveUp(format!("not met after {nudges} nudges: {why}"))
            }
            Verdict::NotMet(why) => Step::Nudge(format!(
                "Not done yet: {why}. Work on it, then send a new report (dash task report) and end your turn."
            )),
            // The judge flagged it to whoever answers: it may still take a word in the pane.
            Verdict::Blocked(_) => Step::Keep,
            Verdict::NotOurs(why) => Step::GiveUp(why.clone()),
        }
    }

    fn retry(&self, attempts: u32) -> bool {
        attempts < ATTEMPTS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_core::tasks::{Held, Limit, OPEN_CAP, Queue, Queued, Standing, plan};
    use dash_meter::memory::TestBudget;

    fn limit(account: &str, five_hours: f64, week: f64, week_resets: u64) -> Limit {
        let account = account.into();
        Limit {
            account,
            five_hours,
            five_hours_resets: 0,
            week,
            week_resets,
            key: false,
        }
    }

    fn snapshot(tasks: &[&str], seats: usize, sessions: usize, limits: Vec<Limit>) -> Snapshot {
        let tasks = tasks.iter().map(|id| Queued {
            held: None,
            id: id.to_string(),
            title: String::new(),
            reopened_ms: None,
            waits: vec![],
        });
        let queue = Queue {
            seats,
            tasks: tasks.collect(),
        };
        Snapshot {
            queue,
            sessions,
            limits,
        }
    }

    #[test]
    fn the_first_task_starts_on_the_account_that_resets_first_with_room() {
        let _budget = TestBudget::new();
        let limits = vec![
            limit("later", 0.0, 10.0, 300),
            limit("spent", 0.0, 100.0, 100),
            limit("hot", 100.0, 50.0, 150),
            limit("next", 99.0, 99.0, 200),
        ];
        let pick = Defaults.pick(&snapshot(&["a", "b"], 5, 4, limits));
        let account = Some("next".to_string());
        let id = "a".to_string();
        assert_eq!(pick, Pick::Start { id, account });
    }

    #[test]
    fn no_seat_no_room_or_nothing_queued_waits_saying_why() {
        let _budget = TestBudget::new();
        let wait = |s| match Defaults.pick(&s) {
            Pick::Wait(why) => why,
            other => panic!("{other:?}"),
        };
        assert!(wait(snapshot(&[], 5, 0, vec![])).contains("nothing"));
        assert!(wait(snapshot(&["a"], 5, 5, vec![])).contains("5 of 5 sessions"));
        let spent = vec![limit("x", 0.0, 100.0, 1), limit("y", 100.0, 0.0, 2)];
        assert!(wait(snapshot(&["a"], 5, 0, spent)).contains("every account"));
    }

    #[test]
    fn a_key_s_row_in_the_snapshot_never_starts_a_pane_nor_stops_one() {
        let _budget = TestBudget::new();
        let key = Limit {
            key: true,
            ..limit("anthropic", 0.0, 10.0, 0)
        };
        let id = "a".to_string();
        let start = Pick::Start { id, account: None };
        let with_room = snapshot(&["a"], 5, 0, vec![key.clone()]);
        assert_eq!(
            Defaults.pick(&with_room),
            start,
            "a key with room is no account"
        );
        let spent = Limit {
            week: 100.0,
            ..key.clone()
        };
        let spent = snapshot(&["a"], 5, 0, vec![spent]);
        assert_eq!(
            Defaults.pick(&spent),
            start,
            "a spent key holds no start back"
        );
        let accounts = vec![limit("x", 0.0, 100.0, 1), key];
        assert!(matches!(
            Defaults.pick(&snapshot(&["a"], 5, 0, accounts)),
            Pick::Wait(_)
        ));
    }

    #[test]
    fn no_reading_of_any_account_does_not_stop_a_start() {
        let _budget = TestBudget::new();
        let id = "a".to_string();
        let account = None;
        assert_eq!(
            Defaults.pick(&snapshot(&["a"], 5, 0, vec![])),
            Pick::Start { id, account }
        );
    }

    fn standing(id: &str, status: &str, outcome: Option<&str>, waits: &[&str]) -> Standing {
        Standing {
            id: id.into(),
            status: status.into(),
            outcome: outcome.map(String::from),
            waits: waits.iter().map(|w| w.to_string()).collect(),
            ..Standing::default()
        }
    }

    #[test]
    fn plan_says_what_runs_next_and_why_each_other_waits() {
        let _budget = TestBudget::new();
        let queue = snapshot(
            &["blocked", "failed-on", "merged-on", "ready", "held"],
            5,
            0,
            vec![],
        )
        .queue;
        let rows = [
            standing("blocked", "open", None, &["running"]),
            standing("failed-on", "open", None, &["gave-up"]),
            standing("merged-on", "open", None, &["landed"]),
            standing("running", "claimed", None, &[]),
            standing("held", "claimed", None, &[]),
            standing("gave-up", "finished", Some("ended"), &[]),
            standing("landed", "finished", Some("merged"), &[]),
            standing("unlisted", "open", None, &[]),
        ];
        let plan = plan(&Defaults, queue, &rows, 0, vec![]);
        let (id, account) = ("merged-on".to_string(), None);
        assert_eq!(plan.next, Pick::Start { id, account }, "its wait is merged");
        let held = |id: &str, why: &str| Held {
            id: id.into(),
            why: why.into(),
        };
        assert_eq!(
            plan.waiting,
            [
                held("ready", "behind merged-on"),
                held("blocked", "waits on running (claimed)"),
                held("failed-on", "waits on gave-up (ended)"),
                held("unlisted", "the list does not offer it"),
            ]
        );
    }

    #[test]
    fn plan_says_the_seats_and_a_merged_loop_waits_without_hanging() {
        let _budget = TestBudget::new();
        let queue = snapshot(&["a", "b"], 5, 5, vec![]).queue;
        let plan_of =
            |sessions, rows: &[Standing]| plan(&Defaults, queue.clone(), rows, sessions, vec![]);
        let full = plan_of(5, &[]);
        assert!(matches!(&full.next, Pick::Wait(why) if why.contains("5 of 5")));
        assert!(full.waiting[0].why.contains("5 of 5"), "{full:?}");
        assert_eq!(full.waiting[1].why, "behind a");
        // Two replicas can each write one edge: each write passed, and the merge holds a loop.
        let rows = [
            standing("a", "open", None, &["b"]),
            standing("b", "open", None, &["a"]),
        ];
        let looped = plan_of(0, &rows);
        assert_eq!(looped.next, Pick::Wait("a: waits on b (open)".into()));
        assert_eq!(looped.waiting[0].why, "waits on b (open)");
    }

    #[test]
    fn at_the_cap_a_task_with_no_row_waits_saying_so_and_an_open_one_runs() {
        let _budget = TestBudget::new();
        let queue = snapshot(&["new", "open-0"], 5, 0, vec![]).queue;
        let rows: Vec<Standing> = (0..OPEN_CAP)
            .map(|n| standing(&format!("open-{n}"), "open", None, &[]))
            .collect();
        let plan = plan(&Defaults, queue, &rows, 0, vec![]);
        assert!(matches!(&plan.next, Pick::Start { id, .. } if id == "open-0"));
        assert!(plan.waiting[0].why.contains("50 Rust tasks are open"));
    }

    #[test]
    fn a_verdict_leads_to_its_step() {
        let _budget = TestBudget::new();
        let no = Verdict::NotMet("part 2".into());
        assert_eq!(Defaults.after(&Verdict::Met, 9, true), Step::Land);
        assert!(matches!(Defaults.after(&no, 2, false), Step::Nudge(t) if t.contains("part 2")));
        assert!(matches!(Defaults.after(&no, 3, false), Step::GiveUp(t) if t.contains("part 2")));
        let blocked = Verdict::Blocked("a tap".into());
        assert_eq!(Defaults.after(&blocked, 0, false), Step::Keep);
        let back = Verdict::NotOurs("handed back".into());
        assert_eq!(
            Defaults.after(&back, 0, true),
            Step::GiveUp("handed back".into())
        );
    }

    #[test]
    fn a_session_past_its_limit_is_judged_and_left_working_unless_met() {
        let _budget = TestBudget::new();
        let no = Verdict::NotMet("part 2".into());
        assert_eq!(Defaults.after(&no, 9, true), Step::Keep);
        assert_eq!(Defaults.after(&Verdict::Met, 0, true), Step::Land);
    }

    #[test]
    fn a_run_tries_three_times() {
        let _budget = TestBudget::new();
        assert!(Defaults.retry(2));
        assert!(!Defaults.retry(3));
    }
}
