//! The stall pass with stand-ins (task rust-stalled-port): what it does to each stalled PR, and when it wakes.
use dash_core::tasks::{Base, Checks, Hold, Pr, Seen, Terminal};
use dash_meter::dash_test;
use dash_orch::stalled::{
    Act, Acts, Holders, On, Open, Pane, act, key, line, on, panes, pass, stalled, text,
};
use std::cell::RefCell;

fn open(n: u64, task: &str, age: i64) -> Open {
    Open {
        pr: Pr {
            number: n,
            url: format!("https://github.com/nha-org/site_perso/pull/{n}"),
            head: format!("head{n}"),
            merged: false,
        },
        branch: format!("task/{task}"),
        task: Some(task.into()),
        head_age_s: age,
    }
}

/// The forge as a script: its base and checks, whether main shares the red, and each act it was asked, in order.
struct Stand {
    base: Base,
    checks: Checks,
    main_red: bool,
    done: RefCell<Vec<String>>,
}

impl Stand {
    fn new(base: Base, checks: Checks, main_red: bool) -> Self {
        Stand {
            base,
            checks,
            main_red,
            done: RefCell::new(vec![]),
        }
    }

    fn act(&self, o: &Open) -> Act {
        let base = |_: &Pr| Ok(self.base.clone());
        let checks = |_: &Pr| Ok(self.checks.clone());
        let main_red = |_: &Pr| Ok(self.main_red);
        let remerge = |o: &Open| {
            self.done
                .borrow_mut()
                .push(format!("remerge {}", o.pr.number));
            Ok("pushed abc".to_string())
        };
        let land = |o: &Open| {
            self.done.borrow_mut().push(format!("land {}", o.pr.number));
            Ok("merged".to_string())
        };
        let acts = Acts {
            base: &base,
            checks: &checks,
            main_red: &main_red,
            remerge: &remerge,
            land: &land,
        };
        act(o, &On::Nobody, &acts)
    }
}

#[dash_test]
fn a_red_that_is_main_s_own_gets_main_merged_in() {
    let s = Stand::new(Base::Behind, Checks::Red("rust (a_test)".into()), true);
    let said = s.act(&open(7, "t7", 3600));
    assert_eq!(*s.done.borrow(), ["remerge 7"]);
    assert_eq!(
        said,
        Act::Moved("found its red main's own (rust (a_test)): main merged in, pushed abc".into())
    );
}

#[dash_test]
fn a_green_pr_with_no_live_run_is_landed_and_one_running_is_left() {
    let s = Stand::new(Base::Holds, Checks::Green, false);
    assert_eq!(
        s.act(&open(8, "t8", 3600)),
        Act::Moved("green with no live run: landed, merged".into())
    );
    assert_eq!(*s.done.borrow(), ["land 8"]);

    let running = Stand::new(Base::Holds, Checks::Pending, false);
    assert_eq!(running.act(&open(9, "t9", 3600)), Act::Runs);
    assert!(
        running.done.borrow().is_empty(),
        "a live run is never landed"
    );
}

#[dash_test]
fn only_a_stall_it_cannot_move_wakes_with_what_it_tried() {
    // its own red: nothing merged, nothing landed, the wake names the red it read
    let own = Stand::new(Base::Behind, Checks::Red("rust (b_test)".into()), false);
    let o = open(10, "t10", 2 * 3600);
    let Act::Wake(tried) = own.act(&o) else {
        panic!("its own red must wake")
    };
    assert!(own.done.borrow().is_empty());
    assert_eq!(
        text(&o, &On::Nobody, &tried),
        "Open PR https://github.com/nha-org/site_perso/pull/10 (branch task/t10) has stalled: head unmoved 2 h 0 min; nobody \
         is on it. Rust's stall watcher read its CI: red on its own code (rust (b_test)), which its task must fix."
    );

    // a conflict: only a session resolves it
    let conflict = Stand::new(Base::Conflicts(vec!["a.rs".into()]), Checks::Green, false);
    assert_eq!(
        conflict.act(&open(11, "t11", 3600)),
        Act::Wake(
            "read its base: it conflicts with main in a.rs, which only a session resolves".into()
        )
    );
    assert!(conflict.done.borrow().is_empty());

    // a land that fails wakes with it
    let land = |_: &Open| Err::<String, String>("refused".into());
    let acts = Acts {
        base: &|_| Ok(Base::Holds),
        checks: &|_| Ok(Checks::Green),
        main_red: &|_| Ok(false),
        remerge: &|_| Ok(String::new()),
        land: &land,
    };
    assert_eq!(
        act(&open(12, "t12", 3600), &On::Nobody, &acts),
        Act::Wake(
            "found it green with no live run, so ran dash land, which failed: refused".into()
        )
    );
}

#[dash_test]
fn the_wake_key_changes_with_a_push_and_every_four_hours() {
    let o = open(13, "t13", 3600);
    assert_eq!(
        key(&o),
        "stalled https://github.com/nha-org/site_perso/pull/13 head13 0"
    );
    let later = Open {
        head_age_s: 5 * 3600,
        ..o.clone()
    };
    assert_ne!(key(&o), key(&later));
}

fn pane(task: &str, idle: bool, shells: bool, question: bool) -> Pane {
    Pane {
        pane: "w1:p2".into(),
        task: task.into(),
        seen: Seen {
            idle,
            asking: question,
            session: None,
        },
        shells,
        question,
    }
}

#[dash_test]
fn a_reviewer_on_its_ci_shell_is_at_work_not_a_stall() {
    // stall-reads-shells' rule: idle, but its footer shows a shell running
    let o = open(14, "t14", 30 * 60);
    let shell = [pane("review-t14", true, true, false)];
    let held = on(&o, &[], &shell, &[], &[]);
    assert_eq!(
        held,
        On::Working("review-t14 waits on its background shell in pane w1:p2".into())
    );
    assert!(
        !stalled(&o, &held),
        "30 minutes on its CI shell is no stall"
    );
    // the same pane with no shell is nobody's after 10 minutes
    let idle = [pane("review-t14", true, false, false)];
    let nobody = on(&o, &[], &idle, &[], &[]);
    assert_eq!(nobody, On::Nobody);
    assert!(stalled(&o, &nobody));
    // an hour tells it whoever holds it
    let old = Open {
        head_age_s: 3600,
        ..o
    };
    assert!(stalled(&old, &held));
}

#[dash_test]
fn a_pr_waiting_on_another_pr_or_his_answer_is_never_a_stall() {
    // stall-reads-blocked-by's rule, each way: a blocked-by naming an open PR, a his: wait, a dialog asking him
    let o = open(15, "t15", 10 * 3600);
    let prs = [
        o.pr.url.clone(),
        "https://github.com/nha-org/site_perso/pull/1476".to_string(),
    ];
    let waits = ["merge of PR 1476 first".to_string()];
    let on_pr = on(&o, &[], &[], &waits, &prs);
    assert_eq!(
        on_pr,
        On::Waits("t15 waits on https://github.com/nha-org/site_perso/pull/1476".into())
    );
    assert!(!stalled(&o, &on_pr));
    // a PR number that only starts the same is not that PR
    let other = ["#14760".to_string()];
    assert_eq!(on(&o, &[], &[], &other, &prs), On::Nobody);

    let his = ["his: pick the layout".to_string()];
    assert!(!stalled(&o, &on(&o, &[], &[], &his, &prs)));

    let asking = [pane("t15", false, false, true)];
    let held = on(&o, &[], &asking, &[], &prs);
    assert_eq!(held, On::Waits("pane w1:p2 asks his answer".into()));
    assert!(!stalled(&o, &held));
}

#[dash_test]
fn a_pr_a_rust_run_claims_is_its_run_s() {
    let o = open(16, "t16", 20 * 60);
    let held = on(&o, &["t16".into()], &[], &[], &[]);
    assert_eq!(held, On::Working("Rust's run of t16".into()));
    assert!(!stalled(&o, &held));
}

#[dash_test]
fn a_pr_someone_works_on_is_woken_on_never_pushed_to_or_landed() {
    // an hour unpushed under a live session: green, yet neither landed nor merged into
    let s = Stand::new(Base::Behind, Checks::Green, false);
    let o = open(17, "t17", 2 * 3600);
    let on = On::Working("t17 is at work in pane w1:p2".into());
    assert!(stalled(&o, &on));
    let acts_seen = RefCell::new(0);
    let base = |_: &Pr| {
        *acts_seen.borrow_mut() += 1;
        Ok(s.base.clone())
    };
    let acts = Acts {
        base: &base,
        checks: &|_| Ok(Checks::Green),
        main_red: &|_| Ok(false),
        remerge: &|_| panic!("never pushed to under a session"),
        land: &|_| panic!("never landed under a session"),
    };
    assert_eq!(
        act(&o, &on, &acts),
        Act::Wake(
            "left it alone, as t17 is at work in pane w1:p2 and its head has not moved in an hour: ask what it waits on, or \
             close it"
                .into()
        )
    );
    assert_eq!(*acts_seen.borrow(), 0, "nothing read either");
}

#[dash_test]
fn a_rust_run_of_a_phase_holds_its_task_s_pr_and_a_longer_pr_number_is_not_a_wait() {
    let o = open(18, "t18", 20 * 60);
    assert_eq!(
        on(&o, &["t18-phase1".into()], &[], &[], &[]),
        On::Working("Rust's run of t18-phase1".into())
    );
    // open PR 147 is not the merged 1476 a wait names
    let prs = [
        o.pr.url.clone(),
        "https://github.com/nha-org/site_perso/pull/147".to_string(),
    ];
    let waits = ["after https://github.com/nha-org/site_perso/pull/1476 merges".to_string()];
    assert_eq!(on(&o, &[], &[], &waits, &prs), On::Nobody);
}

/// A terminal whose every pane is an agent mid-turn.
struct Busy;

impl Terminal for Busy {
    fn open(&self, _: &str, _: &str) -> Result<String, String> {
        Err("no open here".into())
    }
    fn run(&self, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn agent(&self, _: &str) -> Result<Option<Seen>, String> {
        Ok(Some(Seen {
            idle: false,
            asking: false,
            session: None,
        }))
    }
    fn sessions(&self) -> Result<usize, String> {
        Ok(0)
    }
    fn read(&self, _: &str) -> Result<String, String> {
        Ok(String::new())
    }
    fn send(&self, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn key(&self, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn close(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
}

/// /code-review of bfdbdfffb: a folder's task is its last part, a trailing slash read as none, and never a half-made `.new`.
#[dash_test]
fn a_pane_s_task_is_its_folder_s_with_a_trailing_slash_and_never_a_half_made_one() {
    let row = |f: &str| ("p".to_string(), f.to_string());
    let rows = vec![
        row("/w/site_perso/rust-t1/"),
        row("/w/site_perso/rust-t2.new"),
        row("/w/klar/t3"),
    ];
    let tasks: Vec<String> = panes(rows, &Busy).into_iter().map(|p| p.task).collect();
    assert_eq!(tasks, ["t1", "rust-t2.new", "t3"]);
}

/// The pass, end to end on stand-ins: a told stall is skipped, a held PR only woken on, a green one landed, and each wake
/// handed over at once, before a later PR's land runs (/code-review of bfdbdfffb).
#[dash_test]
fn the_pass_skips_a_told_stall_wakes_on_a_held_pr_before_a_later_land_and_says_its_line() {
    let s = Stand::new(Base::Holds, Checks::Green, false);
    let order = RefCell::new(vec![]);
    let base = |_: &Pr| Ok(s.base.clone());
    let checks = |_: &Pr| Ok(s.checks.clone());
    let main_red = |_: &Pr| Ok(false);
    let remerge = |_: &Open| Err("not here".to_string());
    let land = |o: &Open| {
        order.borrow_mut().push(format!("land {}", o.pr.number));
        Ok("merged".to_string())
    };
    let acts = Acts {
        base: &base,
        checks: &checks,
        main_red: &main_red,
        remerge: &remerge,
        land: &land,
    };
    let listed = [
        open(1, "t1", 3 * 3600),
        open(2, "t2", 3 * 3600),
        open(3, "t3", 3600),
    ];
    let held = Holders {
        holds: vec![Hold {
            pr: 2,
            holder: "orchestrator".into(),
            why: "a review".into(),
            touched_ms: 0,
        }],
        ..Holders::default()
    };
    let told_key = key(&listed[0]);
    let told = |k: &str| k == told_key;
    let mut wake = |w: dash_orch::stalled::Wake| {
        order.borrow_mut().push(format!("wake {}", w.number));
        assert!(
            w.title.contains("orchestrator holds it: a review"),
            "{}",
            w.title
        );
        Ok(true)
    };
    let (moved, woke) = pass(&listed, &held, &[], &told, &acts, &mut wake).expect("a pass");
    assert_eq!(*order.borrow(), ["wake 2", "land 3"]);
    assert_eq!(woke, [2]);
    assert_eq!(
        line(listed.len(), &moved, woke.len()),
        "3 open PRs read, 1 moved, 1 woke the orchestrator: #3 green with no live run: landed, merged"
    );
}
