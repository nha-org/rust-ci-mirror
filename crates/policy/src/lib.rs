//! Core's Policy, the default one (section 20b): his seats, his spend order, and what follows a verdict, as the older app's
//! PM check does it. Pure, so a stand-in is only ever needed to test another policy.
pub mod free;

use dash_core::tasks::{
    BUILDS_NOTHING, Demand, Limit, Pick, Policy, Queued, Snapshot, Spend, Step, Verdict, said,
};

/// A PM check's nudges before it gives up (`PM.max_nudges`), and the same for tries at a run.
const NUDGES: u32 = 3;
const ATTEMPTS: u32 = 3;

pub use dash_core::tasks::{BUILD_FREE_PERCENT, LOAD_PER_CORE};

pub struct Defaults;

impl Policy for Defaults {
    fn pick(&self, snapshot: &Snapshot) -> Pick {
        let Some(task) = snapshot.queue.tasks.first() else {
            return Pick::Wait("nothing to pick".into());
        };
        let (open, seats) = (snapshot.sessions, snapshot.queue.seats);
        // a full house says so, and what set the seats (0 seats is `plan_at`'s wait)
        if open >= seats {
            return Pick::Wait(format!(
                "{} waits: every seat is taken, {open} sessions open of {seats} allowed{}",
                task.id,
                said(&snapshot.queue.seats_why)
            ));
        }
        // The first task this moment allows: short of memory, the first that builds nothing.
        let free = snapshot.free_percent;
        let tasks = &snapshot.queue.tasks;
        let Some(task) = tasks.iter().find(|t| self.holds(t, free).is_none()) else {
            let why = self.holds(task, free).unwrap_or_default();
            return Pick::Wait(format!("{} waits: {why}", task.id));
        };
        match spend(task, &snapshot.limits, snapshot.now) {
            Ok((account, pool)) => {
                let id = task.id.clone();
                Pick::Start { id, account, pool }
            }
            Err(why) => Pick::Wait(format!("{} waits: {why}", task.id)),
        }
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

    /// Short of memory for a build, a task that builds is held, saying so; one labelled as building nothing never is.
    fn holds(&self, task: &Queued, free_percent: Option<u8>) -> Option<String> {
        let free = free_percent.filter(|f| *f < BUILD_FREE_PERCENT)?;
        (!task.builds_nothing).then(|| {
            format!(
                "it builds, and {free}% of memory is free, under the {BUILD_FREE_PERCENT}% a build needs: only work \
                 that builds nothing starts (labelled {})",
                BUILDS_NOTHING.join(" or ")
            )
        })
    }
}

/// Each demand's route while the store holds no row for it (@nha, 2026-11-10: « hard tasks go to Fable. We use Fable until it
/// hits the limit. »): a pool is a model `claude --model` takes. A normal task names none: it runs on the login's own model,
/// opus in his settings, as every start did before RFC 20h; `dash orch route normal opus` names it.
pub const DEFAULT_ROUTES: [(Demand, &[&str]); 2] =
    [(Demand::Hard, &["fable", "opus"]), (Demand::Normal, &[])];

/// The default route of `demand`, as owned names.
pub fn default_route(demand: Demand) -> Vec<String> {
    let route = DEFAULT_ROUTES.iter().find(|(d, _)| *d == demand);
    route.map_or(vec![], |(_, r)| r.iter().map(|p| p.to_string()).collect())
}

/// `task`'s route: its row's, else its demand's default.
pub fn route_of(task: &Queued) -> Vec<String> {
    task.route
        .clone()
        .unwrap_or_else(|| default_route(task.demand))
}

/// A pool's refill a hard task waits for rather than spend the next pool (RFC 20h rule 2: Fable refilling within the hour).
pub const REFILL_WAIT: u64 = 3600;

/// When `pool` is back on `login`: the latest reset of what is at its limit. None: no reset read.
fn back_at(login: &Limit, pool: &str, now: u64) -> Option<u64> {
    let own = login.scoped(pool, now).filter(|m| m.used >= 100.0);
    let spent = [
        (login.five_hours >= 100.0).then_some(login.five_hours_resets),
        (login.week >= 100.0).then_some(login.week_resets),
        own.map(|m| m.resets),
    ];
    let spent: Vec<u64> = spent.into_iter().flatten().collect();
    spent
        .iter()
        .all(|r| *r > now)
        .then(|| spent.into_iter().max())
        .flatten()
}

/// The login and pool `task` starts on (RFC 20h rule 1): its route's first pool with room, on the login whose reset of that pool
/// comes first (rule 2, use it or lose it: @nha, 2026-11-01: « use 100% of one sub that expires first before moving to the
/// next »). A pool back within `REFILL_WAIT` is waited for, never passed. No login read: nothing says stop, the first pool.
/// What `dash orch next` says of each task whose route names a pool, seats aside.
pub fn spend(
    task: &Queued,
    limits: &[Limit],
    now: u64,
) -> Result<(Option<String>, Option<Spend>), String> {
    // A key's row is spent by a headless run alone, never a pane's (rust-key-agent's question dialog, 2026-11-08).
    let accounts: Vec<&Limit> = limits.iter().filter(|l| !l.key).collect();
    let accounts = accounts.as_slice();
    let demand = task.demand.name();
    let route = route_of(task);
    if accounts.is_empty() {
        let pool = route.first().map(|pool| Spend {
            pool: pool.clone(),
            why: format!(
                "{demand} task: {pool} is first on its route, and no login's reading says stop"
            ),
        });
        return Ok((None, pool));
    }
    let any_room = accounts.iter().any(|l| l.room());
    if route.is_empty() {
        let mut room: Vec<_> = accounts.iter().filter(|l| l.room()).collect();
        room.sort_by_key(|l| l.week_resets);
        let first = room.first().ok_or("every account is at its limit")?;
        return Ok((Some(first.account.clone()), None));
    }
    let mut passed: Vec<String> = vec![];
    for pool in &route {
        let resets = |l: &&&Limit| l.scoped(pool, now).map_or(l.week_resets, |m| m.resets);
        let mut room: Vec<_> = accounts
            .iter()
            .filter(|l| l.room_for(Some(pool), now))
            .collect();
        room.sort_by_key(resets);
        if let Some(login) = room.first() {
            let used = login.scoped(pool, now).map_or(String::new(), |m| {
                format!(", {}% of its own allowance used", m.used)
            });
            let why = match passed.is_empty() {
                true => format!(
                    "{demand} task: {pool} is first on its route and has room on {}{used}",
                    login.account
                ),
                false => format!(
                    "{demand} task: {} at its limit on every login, so {pool}, next on its route, on {}{used}",
                    passed.join(" and "),
                    login.account
                ),
            };
            let pool = Some(Spend {
                pool: pool.clone(),
                why,
            });
            return Ok((Some(login.account.clone()), pool));
        }
        let back = accounts.iter().filter_map(|l| back_at(l, pool, now)).min();
        if let Some(back) = back.filter(|b| b - now <= REFILL_WAIT) {
            let minutes = (back - now).div_ceil(60);
            return Err(format!(
                "{pool} is at its limit and refills in {minutes} min: a {demand} task waits for it rather than spend the next pool"
            ));
        }
        passed.push(pool.clone());
    }
    match any_room {
        false => Err("every account is at its limit".into()),
        true => Err(format!(
            "no pool on its {demand} route has room: {}",
            route.join(", ")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_core::tasks::{Held, Limit, OPEN_CAP, Queue, Queued, Standing, plan};
    use dash_meter::dash_test;

    fn limit(account: &str, five_hours: f64, week: f64, week_resets: u64) -> Limit {
        let account = account.into();
        Limit {
            account,
            five_hours,
            five_hours_resets: 0,
            week,
            week_resets,
            key: false,
            models: vec![],
        }
    }

    fn snapshot(tasks: &[&str], seats: usize, sessions: usize, limits: Vec<Limit>) -> Snapshot {
        let tasks = tasks.iter().map(|id| Queued {
            held: None,
            id: id.to_string(),
            title: String::new(),
            reopened_ms: None,
            waits: vec![],
            layer: None,
            builds_nothing: id.starts_with("docs-"),
            rank: Default::default(),
            free: None,
            demand: Default::default(),
            route: None,
            ..Queued::default()
        });
        let queue = Queue {
            seats,
            seats_why: String::new(),
            tasks: tasks.collect(),
        };
        Snapshot {
            queue,
            sessions,
            limits,
            free_percent: None,
            now: 0,
        }
    }

    /// Task rust-orch-pick-dynamic, done-when 2: short of free memory, only a task that builds nothing starts; with none queued,
    /// the pick waits saying why in plain words. Memory unread or at the line holds nothing back.
    #[dash_test]
    fn short_of_memory_only_a_task_that_builds_nothing_starts() {
        let at = |free: Option<u8>, tasks: &[&str]| {
            let s = snapshot(tasks, 5, 0, vec![]);
            Defaults.pick(&Snapshot {
                free_percent: free,
                ..s
            })
        };
        let start = |id: &str| Pick::Start {
            id: id.into(),
            account: None,
            pool: None,
        };
        let low = Some(BUILD_FREE_PERCENT - 1);
        assert_eq!(at(low, &["build", "docs-plan"]), start("docs-plan"));
        assert_eq!(
            at(low, &["build", "other"]),
            Pick::Wait(
                "build waits: it builds, and 24% of memory is free, under the 25% a build needs: only work that builds \
                 nothing starts (labelled Docs or Research)"
                    .into()
            )
        );
        assert_eq!(at(Some(BUILD_FREE_PERCENT), &["build"]), start("build"));
        assert_eq!(at(None, &["build"]), start("build"));
        // the plan says the same: the task that builds waits, its reason the pick's
        let s = snapshot(&["build", "docs-plan"], 5, 0, vec![]);
        let plan = plan(&Defaults, s.queue, &[], 0, vec![], low);
        assert_eq!(plan.next, start("docs-plan"));
        assert_eq!(plan.waiting[0].id, "build");
        assert!(
            plan.waiting[0]
                .why
                .starts_with("it builds, and 24% of memory is free")
        );
    }

    /// A login with its week read and Fable's own allowance at `fable` percent, both resetting at `resets`.
    fn login(account: &str, week: f64, fable: Option<f64>, resets: u64) -> Limit {
        let fable = fable.map(|used| dash_core::tasks::Scoped {
            model: "fable".into(),
            used,
            resets,
        });
        Limit {
            models: fable.into_iter().collect(),
            ..limit(account, 0.0, week, resets)
        }
    }

    /// `tasks` queued with their demand, `hard-` ones hard, each on its default route, at `now`.
    fn demanding(tasks: &[&str], limits: Vec<Limit>, now: u64) -> Snapshot {
        let mut s = snapshot(tasks, 5, 0, limits);
        for t in &mut s.queue.tasks {
            t.demand = match t.id.starts_with("hard-") {
                true => dash_core::tasks::Demand::Hard,
                false => dash_core::tasks::Demand::Normal,
            };
        }
        Snapshot { now, ..s }
    }

    fn started(pick: Pick) -> (String, Option<String>, String, String) {
        match pick {
            Pick::Start {
                id,
                account,
                pool: Some(p),
            } => (id, account, p.pool, p.why),
            other => panic!("{other:?}"),
        }
    }

    /// Task rust-pools-fable, done-when 2, RFC 20h rule 1 (the pick is one step): the order picks the task, its route the pool,
    /// the first with room, and the login under it by 20g's rule. A hard task goes to Fable while Fable has room; a normal one to
    /// Opus; Fable at its limit on every login, with two hard tasks queued, sends the first to Opus, saying why.
    #[dash_test]
    fn rule_20h_1_a_hard_task_takes_fable_while_it_has_room_then_opus() {
        let now = 1_000;
        let day = now + 86_400;
        let room = vec![
            login("later", 10.0, Some(20.0), day + 60),
            login("first", 10.0, Some(40.0), day),
        ];
        let (id, account, pool, why) =
            started(Defaults.pick(&demanding(&["hard-a"], room.clone(), now)));
        assert_eq!(
            (id.as_str(), account.as_deref(), pool.as_str()),
            ("hard-a", Some("first"), "fable")
        );
        assert_eq!(
            why,
            "hard task: fable is first on its route and has room on first, 40% of its own allowance used"
        );
        // a normal task's route names no pool: the login's own model, on 20g's login
        let normal = Defaults.pick(&demanding(&["normal"], room.clone(), now));
        let (id, account) = ("normal".to_string(), Some("first".to_string()));
        assert_eq!(
            normal,
            Pick::Start {
                id,
                account,
                pool: None
            }
        );
        // a route set to opus is spent as any pool
        let mut set = demanding(&["normal"], room, now);
        set.queue.tasks[0].route = Some(vec!["opus".into()]);
        let (_, account, pool, why) = started(Defaults.pick(&set));
        assert_eq!((account.as_deref(), pool.as_str()), (Some("first"), "opus"));
        assert_eq!(
            why,
            "normal task: opus is first on its route and has room on first"
        );
        // Fable spent on every login for a day: both hard tasks would wait, so the first runs on Opus
        let spent = vec![
            login("later", 10.0, Some(100.0), day + 60),
            login("first", 10.0, Some(100.0), day),
        ];
        let (id, account, pool, why) =
            started(Defaults.pick(&demanding(&["hard-a", "hard-b"], spent, now)));
        assert_eq!(
            (id.as_str(), account.as_deref(), pool.as_str()),
            ("hard-a", Some("first"), "opus")
        );
        assert_eq!(
            why,
            "hard task: fable at its limit on every login, so opus, next on its route, on first"
        );
        // a login with Fable left takes it even when another's week resets sooner
        let mixed = vec![
            login("first", 10.0, Some(100.0), day),
            login("later", 10.0, Some(30.0), day + 60),
        ];
        let (_, account, pool, _) = started(Defaults.pick(&demanding(&["hard-a"], mixed, now)));
        assert_eq!(
            (account.as_deref(), pool.as_str()),
            (Some("later"), "fable")
        );
        // no login read: nothing says stop, the first pool
        let (_, account, pool, why) = started(Defaults.pick(&demanding(&["hard-a"], vec![], now)));
        assert_eq!((account, pool.as_str()), (None, "fable"));
        assert!(why.contains("no login's reading says stop"), "{why}");
        // every login out for the week: no pool has room
        let out = vec![login("first", 100.0, Some(0.0), day)];
        assert_eq!(
            Defaults.pick(&demanding(&["hard-a"], out, now)),
            Pick::Wait("hard-a waits: every account is at its limit".into())
        );
    }

    /// Task rust-pools-fable, done-when 2, RFC 20h rule 2 (use it or lose it): the pool resetting sooner is spent first; a hard task
    /// waits for Fable only when it refills within the hour, else runs on Opus; an allowance past its reset is whole again.
    #[dash_test]
    fn rule_20h_2_the_sooner_reset_is_spent_first_and_fable_is_waited_for_only_within_the_hour() {
        let now = 1_000;
        // Fable's own reset comes before the login's week: its reset orders the logins for a Fable start
        let soon = Limit {
            models: vec![dash_core::tasks::Scoped {
                model: "fable".into(),
                used: 50.0,
                resets: now + 600,
            }],
            ..limit("week-later", 0.0, 10.0, now + 90_000)
        };
        let other = login("week-sooner", 10.0, Some(50.0), now + 80_000);
        let (_, account, pool, _) =
            started(Defaults.pick(&demanding(&["hard-a"], vec![other, soon], now)));
        assert_eq!(
            (account.as_deref(), pool.as_str()),
            (Some("week-later"), "fable")
        );
        // Fable back in 40 minutes: the hard task waits rather than spend Opus (the live ByKind then tries the next task)
        let back = vec![login("first", 10.0, Some(100.0), now + 2_400)];
        let s = demanding(&["hard-a"], back.clone(), now);
        assert_eq!(
            Defaults.pick(&s),
            Pick::Wait(
                "hard-a waits: fable is at its limit and refills in 40 min: a hard task waits for it rather than spend the \
                 next pool"
                    .into()
            )
        );
        // past the hour: Opus
        let later = vec![login("first", 10.0, Some(100.0), now + REFILL_WAIT + 1)];
        let (_, _, pool, _) = started(Defaults.pick(&demanding(&["hard-a"], later, now)));
        assert_eq!(pool, "opus");
        // an allowance read at 100 % whose reset has passed is whole again
        let passed = Limit {
            week_resets: now + 9_000,
            ..login("first", 10.0, Some(100.0), now - 1)
        };
        let (_, _, pool, _) = started(Defaults.pick(&demanding(&["hard-a"], vec![passed], now)));
        assert_eq!(pool, "fable");
    }

    #[dash_test]
    fn the_first_task_starts_on_the_account_that_resets_first_with_room() {
        let limits = vec![
            limit("later", 0.0, 10.0, 300),
            limit("spent", 0.0, 100.0, 100),
            limit("hot", 100.0, 50.0, 150),
            limit("next", 99.0, 99.0, 200),
        ];
        let pick = Defaults.pick(&snapshot(&["a", "b"], 5, 4, limits));
        let account = Some("next".to_string());
        let id = "a".to_string();
        assert_eq!(
            pick,
            Pick::Start {
                id,
                account,
                pool: None
            }
        );
    }

    /// Task rust-graph-queue-edges, done-when 3 and 4: no seat allowed and a full house each say so in plain words, never
    /// « 1 of 0 sessions open », which reads as a dependency.
    #[dash_test]
    fn a_wait_on_seats_says_none_allowed_or_all_taken_in_plain_words() {
        let wait = |s| match Defaults.pick(&s) {
            Pick::Wait(why) => why,
            other => panic!("{other:?}"),
        };
        // 0 seats is `plan_at`'s wait, whatever the policy (PR 1463's /code-review)
        let s = snapshot(&["a"], 0, 1, vec![]);
        let Pick::Wait(none) = plan(&Defaults, s.queue, &[], 1, vec![], None).next else {
            panic!("no seat, no start")
        };
        assert_eq!(none, "a waits: no session may start, 0 seats allowed");
        let full = wait(snapshot(&["a"], 2, 2, vec![]));
        assert_eq!(
            full,
            "a waits: every seat is taken, 2 sessions open of 2 allowed"
        );
        let mut capped = snapshot(&["a"], 2, 2, vec![]);
        // dash_store::older::seats' wording; policy does not depend on the store
        let why = "day 5, capped at 2 by rust-seats, a setting that never expires (`dash project set site_perso rust-seats` clears it)";
        capped.queue.seats_why = why.into();
        assert!(
            wait(capped).ends_with(&format!("of 2 allowed ({why})")),
            "a full house names what set the seats"
        );
        assert!(!none.contains(" of 0") && !full.contains(" of 0"));
    }

    #[dash_test]
    fn no_seat_no_room_or_nothing_queued_waits_saying_why() {
        let wait = |s| match Defaults.pick(&s) {
            Pick::Wait(why) => why,
            other => panic!("{other:?}"),
        };
        assert!(wait(snapshot(&[], 5, 0, vec![])).contains("nothing"));
        assert!(wait(snapshot(&["a"], 5, 5, vec![])).contains("5 sessions open of 5 allowed"));
        let spent = vec![limit("x", 0.0, 100.0, 1), limit("y", 100.0, 0.0, 2)];
        assert!(wait(snapshot(&["a"], 5, 0, spent)).contains("every account"));
    }

    #[dash_test]
    fn a_key_s_row_in_the_snapshot_never_starts_a_pane_nor_stops_one() {
        let key = Limit {
            key: true,
            ..limit("anthropic", 0.0, 10.0, 0)
        };
        let id = "a".to_string();
        let start = Pick::Start {
            id,
            account: None,
            pool: None,
        };
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

    #[dash_test]
    fn no_reading_of_any_account_does_not_stop_a_start() {
        let id = "a".to_string();
        let account = None;
        assert_eq!(
            Defaults.pick(&snapshot(&["a"], 5, 0, vec![])),
            Pick::Start {
                id,
                account,
                pool: None
            }
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

    #[dash_test]
    fn plan_says_what_runs_next_and_why_each_other_waits() {
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
        let plan = plan(&Defaults, queue, &rows, 0, vec![], None);
        let (id, account) = ("merged-on".to_string(), None);
        assert_eq!(
            plan.next,
            Pick::Start {
                id,
                account,
                pool: None
            },
            "its wait is merged"
        );
        let held = |id: &str, why: &str| Held {
            id: id.into(),
            why: why.into(),
            queued: id == "ready",
        };
        assert_eq!(
            plan.waiting,
            [
                held("ready", "queued"),
                held("blocked", "waits on running (claimed)"),
                held("failed-on", "waits on gave-up (ended)"),
                held("unlisted", "the list does not offer it"),
            ]
        );
    }

    #[dash_test]
    fn plan_says_the_seats_and_a_merged_loop_waits_without_hanging() {
        let queue = snapshot(&["a", "b"], 5, 5, vec![]).queue;
        let plan_of = |sessions, rows: &[Standing]| {
            plan(&Defaults, queue.clone(), rows, sessions, vec![], None)
        };
        let full = plan_of(5, &[]);
        assert!(
            matches!(&full.next, Pick::Wait(why) if why.contains("5 sessions open of 5 allowed"))
        );
        assert!(
            full.waiting[0].why.contains("5 sessions open of 5 allowed"),
            "{full:?}"
        );
        assert_eq!(full.waiting[1].why, "queued");
        // Two replicas can each write one edge: each write passed, and the merge holds a loop.
        let rows = [
            standing("a", "open", None, &["b"]),
            standing("b", "open", None, &["a"]),
        ];
        let looped = plan_of(0, &rows);
        assert_eq!(looped.next, Pick::Wait("a: waits on b (open)".into()));
        assert_eq!(looped.waiting[0].why, "waits on b (open)");
    }

    #[dash_test]
    fn at_the_cap_a_task_with_no_row_waits_saying_so_and_an_open_one_runs() {
        let queue = snapshot(&["new", "open-0"], 5, 0, vec![]).queue;
        let rows: Vec<Standing> = (0..OPEN_CAP)
            .map(|n| standing(&format!("open-{n}"), "open", None, &[]))
            .collect();
        let plan = plan(&Defaults, queue, &rows, 0, vec![], None);
        assert!(matches!(&plan.next, Pick::Start { id, .. } if id == "open-0"));
        assert!(plan.waiting[0].why.contains("50 Rust tasks are open"));
    }

    #[dash_test]
    fn a_verdict_leads_to_its_step() {
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

    #[dash_test]
    fn a_session_past_its_limit_is_judged_and_left_working_unless_met() {
        let no = Verdict::NotMet("part 2".into());
        assert_eq!(Defaults.after(&no, 9, true), Step::Keep);
        assert_eq!(Defaults.after(&Verdict::Met, 0, true), Step::Land);
    }

    #[dash_test]
    fn a_run_tries_three_times() {
        assert!(Defaults.retry(2));
        assert!(!Defaults.retry(3));
    }
}
