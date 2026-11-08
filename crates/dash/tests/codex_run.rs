//! Section 20b phase 3: a Codex task's whole run through the unchanged orch, on the real store, Codex agent and Codex launch
//! line, its pane the one recorded 2026-11-08 (dash-agent's fixtures/codex): dialogs, a session herdr names only once its first
//! turn starts, the report read from its rollout, its outcome recorded.
use dash_agent::codex::Codex;
use dash_core::tasks::{Checks, Forge, Outcome, Pr, Queue, Seen, TaskList, Terminal, Verdict};
use dash_launcher::ApprovedCodex;
use dash_meter::TestDir;
use dash_meter::memory::TestBudget;
use dash_orch::{Pace, Ports, run_task};
use dash_policy::Defaults;
use dash_store::tasks::open;
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::time::Duration;

const PACE: Pace = Pace {
    poll: Duration::ZERO,
    start_limit: Duration::from_secs(5),
    limit: Duration::from_secs(5),
    stuck_limit: Duration::from_secs(5),
    stop_polls: 3,
    ci_poll: Duration::ZERO,
    ci_limit: Duration::from_secs(5),
    judge_poll: Duration::ZERO,
    patient_poll: Duration::ZERO,
    limit_retry: Duration::ZERO,
};

/// The recorded session's id, as herdr listed it.
const SESSION: &str = "01a1bc43-39c8-7be0-ae2c-15d36567944d";

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../agent/fixtures/codex")
        .join(name);
    std::fs::read_to_string(path).expect("fixture")
}

/// The pane: each recorded screen until the key it needs, herdr's listing, and what was typed.
struct Pane {
    screens: Vec<(String, Option<&'static str>)>,
    at: Cell<usize>,
    did: RefCell<Vec<String>>,
    looks: Cell<u32>,
    judged: RefCell<Vec<String>>,
}

impl Pane {
    fn typed(&self, prefix: &str) -> Vec<String> {
        let did = self.did.borrow();
        did.iter()
            .filter(|d| d.starts_with(prefix))
            .cloned()
            .collect()
    }
}

impl Terminal for Pane {
    fn open(&self, _folder: &str, _label: &str) -> Result<String, String> {
        Ok("p1".into())
    }
    fn run(&self, _pane: &str, line: &str) -> Result<(), String> {
        self.did.borrow_mut().push(format!("run {line}"));
        Ok(())
    }
    /// No id before its first turn, as recorded; idle once briefed, since its rollout holds the turn ended; gone once told /exit.
    fn agent(&self, _pane: &str) -> Result<Option<Seen>, String> {
        if !self.typed("send /exit").is_empty() {
            return Ok(None);
        }
        let look = self.looks.get();
        self.looks.set(look + 1);
        let session = (look > 0).then(|| SESSION.to_string());
        let idle = !self.typed("send the brief").is_empty();
        Ok(Some(Seen {
            idle,
            asking: false,
            session,
        }))
    }
    fn sessions(&self) -> Result<usize, String> {
        Ok(0)
    }
    fn read(&self, _pane: &str) -> Result<String, String> {
        Ok(self.screens[self.at.get()].0.clone())
    }
    fn send(&self, _pane: &str, text: &str) -> Result<(), String> {
        self.did.borrow_mut().push(format!("send {text}"));
        Ok(())
    }
    fn key(&self, _pane: &str, key: &str) -> Result<(), String> {
        let at = self.at.get();
        if self.typed("send").is_empty() {
            assert_eq!(Some(key), self.screens[at].1, "screen {at}");
            self.at.set(at + 1);
        }
        self.did.borrow_mut().push(format!("key {key}"));
        Ok(())
    }
    fn close(&self, pane: &str) -> Result<(), String> {
        self.did.borrow_mut().push(format!("close {pane}"));
        Ok(())
    }
}

impl TaskList for Pane {
    fn queue(&self) -> Result<Queue, String> {
        Err("not read by a run".into())
    }
    fn brief(&self, id: &str) -> Result<String, String> {
        Ok(format!("the brief of {id}"))
    }
    fn judge(&self, _id: &str, said: &str) -> Result<Verdict, String> {
        self.judged.borrow_mut().push(said.to_string());
        Ok(Verdict::Met)
    }
    fn merged(&self, _id: &str, _pr: &str) -> Result<(), String> {
        Ok(())
    }
    fn ended(&self, _id: &str, _why: &str) -> Result<(), String> {
        Ok(())
    }
    fn flag(&self, _id: &str, _what: &str) -> Result<(), String> {
        Ok(())
    }
    fn waiting(&self, _id: &str, _on: Option<&Outcome>) -> Result<(), String> {
        Ok(())
    }
}

impl Forge for Pane {
    fn worktree(&self, id: &str) -> Result<String, String> {
        Ok(format!("/work/rust-{id}"))
    }
    fn pr(&self, _id: &str) -> Result<Option<Pr>, String> {
        let pr = Pr {
            number: 7,
            url: "https://pr/7".into(),
            head: "abc".into(),
            merged: false,
        };
        Ok((!self.typed("send the brief").is_empty()).then_some(pr))
    }
    fn checks(&self, _pr: &Pr) -> Result<Checks, String> {
        Ok(Checks::Green)
    }
    fn stale(&self, _pr: &Pr) -> Result<Vec<String>, String> {
        Ok(vec![])
    }
    fn merge(&self, _pr: &Pr) -> Result<(), String> {
        Ok(())
    }
    fn leave(&self, _id: &str) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn a_codex_task_starts_codex_in_its_pane_and_its_outcome_is_recorded() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("codex-run");
    // Its rollout up to its first turn's end: the report « ok, TASK_DONE ».
    let day = dir.0.join("sessions/2026/11/08");
    std::fs::create_dir_all(&day).expect("dirs");
    let turn: Vec<String> = fixture("rollout.jsonl")
        .lines()
        .take(4)
        .map(String::from)
        .collect();
    let rollout = day.join(format!("rollout-2026-11-08T18-44-44-{SESSION}.jsonl"));
    std::fs::write(rollout, turn.join("\n")).expect("rollout");
    let skip = fixture("update.txt")
        .replace("› 1. Update", "  1. Update")
        .replace("  2. Skip\n", "› 2. Skip\n");
    let screens = vec![
        (fixture("update.txt"), Some("Down")),
        (skip, Some("Enter")),
        (fixture("trust.txt"), Some("Enter")),
        (fixture("hooks.txt"), Some("Down")),
        (fixture("hooks-down.txt"), Some("Enter")),
        (fixture("ready.txt"), None),
    ];
    let pane = Pane {
        screens,
        at: Cell::new(0),
        did: RefCell::default(),
        looks: Cell::new(0),
        judged: RefCell::default(),
    };
    let codex = Codex {
        bin: "codex".into(),
        sessions: dir.0.join("sessions"),
    };
    let opener = dash_agent::codex::OPENER;
    let launcher = ApprovedCodex {
        home: "/h".into(),
        opener,
    };
    let mut tasks = open(&dir.0, &dir.0).expect("the store");
    let mut ports = Ports {
        tasks: &mut tasks,
        list: &pane,
        terminal: &pane,
        launcher: &launcher,
        agent: &codex,
        forge: &pane,
        policy: &Defaults,
        step: &|_, _| {},
    };
    let run = run_task("t1", Some("second"), &mut ports, PACE).expect("recorded");

    let line = &pane.typed("run ")[0];
    assert!(
        line.contains(" codex CODEX_ARGS='-m gpt-5.6-terra "),
        "{line}"
    );
    assert!(!line.contains("CLAUDE_ACCOUNT"), "{line}");
    assert!(
        line.contains(opener),
        "its opener, the one its outcome reads past: {line}"
    );
    assert_eq!(pane.at.get(), 5, "every dialog answered");
    assert_eq!(
        pane.typed("send the brief"),
        ["send the brief of t1"],
        "briefed once named"
    );
    assert_eq!(
        *pane.judged.borrow(),
        ["ok\nTASK_DONE"],
        "its rollout's report judged"
    );
    assert_eq!(run.session.as_deref(), Some(SESSION));
    assert_eq!(run.outcome, Outcome::Merged("https://pr/7".into()));
    let row = tasks.show("t1").expect("show").expect("row");
    assert!(
        row.contains("status: finished") && row.contains("outcome: merged"),
        "{row}"
    );
    assert!(row.contains(&format!("session: {SESSION}")), "{row}");
    assert!(!pane.typed("send /exit").is_empty(), "told to leave");
}
