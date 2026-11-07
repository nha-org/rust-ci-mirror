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
        let mut room: Vec<_> = snapshot.limits.iter().filter(|l| l.room()).collect();
        room.sort_by_key(|l| l.week_resets);
        if room.is_empty() && !snapshot.limits.is_empty() {
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
    use dash_core::tasks::{Limit, Queue, Queued};
    use dash_meter::memory::TestBudget;

    fn limit(account: &str, five_hours: f64, week: f64, week_resets: u64) -> Limit {
        let account = account.into();
        Limit {
            account,
            five_hours,
            week,
            week_resets,
        }
    }

    fn snapshot(tasks: &[&str], seats: usize, sessions: usize, limits: Vec<Limit>) -> Snapshot {
        let tasks = tasks.iter().map(|id| Queued {
            id: id.to_string(),
            title: String::new(),
            reopened_ms: None,
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
    fn no_reading_of_any_account_does_not_stop_a_start() {
        let _budget = TestBudget::new();
        let id = "a".to_string();
        let account = None;
        assert_eq!(
            Defaults.pick(&snapshot(&["a"], 5, 0, vec![])),
            Pick::Start { id, account }
        );
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
