//! A task pane's failure read from its screen (task pane-fail-detect; @nha, 2026-11-02: « I asked for such a status check a
//! bunch of times fyi, we need to reliably detect failures / stuck panes automatically and alert the orchestrator »): the
//! stuck watch waited 5 h of silence while 9 vibe panes printed their failure at once and sat 3 h. Each pass reads every
//! claimed pane's last lines against `pane-signatures.json`, one row per incident that taught it; a match is a watch row of
//! kind `Kind::Pane`, due at once. A screen showing a running tool call is working whatever a line above it says.
use crate::watch::{Item, Kind};
use dash_core::OpLog;
use dash_core::tasks::{Status, Terminal};
use serde_json::Value as Json;
use std::collections::BTreeMap;

/// The list, tracked beside this code: a new failure seen adds one row, with its incident.
pub const SIGNATURES: &str = include_str!("pane-signatures.json");

/// What a pane's screen says.
#[derive(Clone, Debug, PartialEq)]
pub enum Read {
    /// A tool call or a turn runs: nothing to wake on, nothing to release.
    Working,
    /// A failure signature matched: its name and the line that matched.
    Failed {
        name: String,
        line: String,
    },
    /// A done line shows and nothing runs.
    Done,
    Quiet,
}

/// The signature list, parsed.
#[derive(Clone, Debug)]
pub struct Signatures {
    lines: usize,
    working: Vec<String>,
    done: Vec<String>,
    fails: Vec<Fail>,
}

#[derive(Clone, Debug)]
struct Fail {
    name: String,
    /// Every one of these in one line of the foot, which starts with `starts` when it is set.
    has: Vec<String>,
    starts: Option<String>,
    /// The foot's last line ends with one of these, or is one trimmed.
    last_ends: Vec<String>,
}

fn strings(j: &Json, key: &str) -> Vec<String> {
    (j[key].as_array().into_iter().flatten())
        .filter_map(|s| s.as_str().map(String::from))
        .collect()
}

impl Signatures {
    pub fn parse(text: &str) -> Result<Signatures, String> {
        let j: Json =
            serde_json::from_str(text).map_err(|e| format!("the pane signatures: {e}"))?;
        let fails = (j["fails"].as_array())
            .ok_or("the pane signatures: no fails")?
            .iter()
            .map(|f| {
                let name = f["name"].as_str().ok_or("a pane signature has no name")?;
                let fail = Fail {
                    name: name.into(),
                    has: strings(f, "has"),
                    starts: f["starts"].as_str().map(String::from),
                    last_ends: strings(f, "last_ends"),
                };
                if fail.has.is_empty() == fail.last_ends.is_empty() {
                    return Err(format!("pane signature {name}: one of has or last_ends"));
                }
                Ok(fail)
            })
            .collect::<Result<_, String>>()?;
        Ok(Signatures {
            lines: j["lines"].as_u64().unwrap_or(15) as usize,
            working: strings(&j, "working"),
            done: strings(&j, "done"),
            fails,
        })
    }

    /// The tracked list, parsed once.
    pub fn tracked() -> &'static Signatures {
        static TRACKED: std::sync::LazyLock<Signatures> = std::sync::LazyLock::new(|| {
            Signatures::parse(SIGNATURES).expect("pane-signatures.json parses: its test holds it")
        });
        &TRACKED
    }

    /// The screen's last `lines` lines, blank ones left out. A working word below the lowest failure line is a run that went
    /// on after it: working. Else the lowest failure line wins, a bare prompt only when no line matched; a bare prompt under a
    /// done line is a finished run, done.
    pub fn read(&self, screen: &str) -> Read {
        let all: Vec<&str> = screen.lines().filter(|l| !l.trim().is_empty()).collect();
        let foot = &all[all.len().saturating_sub(self.lines)..];
        let says = |l: &str, words: &[String]| words.iter().any(|w| l.contains(w.as_str()));
        let done = foot.iter().any(|l| says(l, &self.done));
        let Some(last) = foot.last().map(|l| l.trim()) else {
            return Read::Quiet;
        };
        let mut hit: Option<(usize, &Fail, &str)> = None;
        // a line's own words first: a bare prompt below them is their symptom, named only when nothing else matched
        let (words, ends): (Vec<&Fail>, Vec<&Fail>) =
            self.fails.iter().partition(|f| !f.has.is_empty());
        for f in words.into_iter().chain(ends) {
            let found = if f.has.is_empty() {
                let ends = |e: &String| last.ends_with(e.as_str()) || last == e.trim();
                (!done && hit.is_none() && f.last_ends.iter().any(ends))
                    .then_some((foot.len() - 1, last))
            } else {
                let starts = |l: &str| {
                    f.starts
                        .as_deref()
                        .is_none_or(|s| l.trim_start().starts_with(s))
                };
                (foot.iter().enumerate().rev())
                    .find(|(_, l)| starts(l) && f.has.iter().all(|w| l.contains(w.as_str())))
                    .map(|(i, l)| (i, l.trim()))
            };
            if let Some((i, line)) = found
                && hit.is_none_or(|(j, _, _)| i > j)
            {
                hit = Some((i, f, line));
            }
        }
        let below = hit.map_or(0, |(i, _, _)| i + 1);
        if foot[below..].iter().any(|l| says(l, &self.working)) {
            return Read::Working;
        }
        match hit {
            Some((_, f, line)) => Read::Failed {
                name: f.name.clone(),
                line: line.into(),
            },
            None if done => Read::Done,
            None => Read::Quiet,
        }
    }
}

/// A failed pane's watch row: one per pane, task and signature, so a pane reused by another task fails anew.
/// ponytail: the same task failing the same way twice in one pane wakes once; its outcome row says what was done.
pub fn row(pane: &str, task: &str, name: &str, line: &str, now_ms: i64) -> Item {
    let key = format!("pane {pane} {task} {name}");
    let title = format!("task {task}'s pane {pane} shows {name}");
    Item::new(&key, Kind::Pane, Some(task), &title, line, now_ms)
}

/// Each claimed pane (`tasks`, pane to task id) whose last lines match a failure signature, as a watch row due at once. A
/// pane `statuses` reads as working is not read: its agent runs. A pane that does not read is skipped this pass, never a
/// failure.
/// ponytail: one read per other claimed pane per pass, one after another, each bounded by herdr's 15 s; a thread each if
/// failed panes pass a dozen.
pub fn failures(
    terminal: &dyn Terminal,
    tasks: &BTreeMap<String, String>,
    statuses: &[Status],
    signatures: &Signatures,
    now_ms: i64,
) -> Vec<Item> {
    let working =
        |pane: &String| (statuses.iter()).any(|s| &s.pane == pane && s.state == "working");
    (tasks.iter())
        .filter(|(pane, _)| !working(pane))
        .filter_map(
            |(pane, task)| match signatures.read(&terminal.read(pane).ok()?) {
                Read::Failed { name, line } => Some(row(pane, task, &name, &line, now_ms)),
                _ => None,
            },
        )
        .collect()
}

/// Lands each of `items`, once by its key, in one read and one write: how many are new.
pub fn land(log: &mut dyn OpLog, items: Vec<Item>) -> Result<usize, String> {
    let kept = crate::watch::load(log)?;
    let mut rows = kept.clone();
    let new: Vec<Item> = (items.into_iter())
        .filter(|i| !kept.contains_key(&i.key))
        .collect();
    let n = new.len();
    rows.extend(new.into_iter().map(|i| (i.key.clone(), i)));
    crate::watch::save(log, &kept, &rows).map(|_| n)
}

/// One pass of the comment watcher over the claimed panes, the tracked signatures read: how many failures landed, a fault
/// said on stderr.
pub fn pass(
    log: &mut dyn OpLog,
    terminal: &dyn Terminal,
    tasks: &BTreeMap<String, String>,
    statuses: &[Status],
    now_ms: i64,
) -> usize {
    let found = failures(terminal, tasks, statuses, Signatures::tracked(), now_ms);
    land(log, found).unwrap_or_else(|e| {
        eprintln!("comment: the pane failures: {e}");
        0
    })
}

/// Whether `pane`'s screen shows a running tool call; a screen that does not read shows none. herdr may read a run as done
/// or gone while its tool runs, so a release waits on the screen too (`dash`'s done holds).
pub fn running(terminal: &dyn Terminal, pane: &str) -> bool {
    (terminal.read(pane)).is_ok_and(|s| Signatures::tracked().read(&s) == Read::Working)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    fn read(screen: &str) -> Read {
        Signatures::tracked().read(screen)
    }

    fn failed(name: &str) -> impl Fn(&Read) -> bool + '_ {
        move |r| matches!(r, Read::Failed { name: n, .. } if n == name)
    }

    /// Each pane's screen; a pane not here does not read.
    struct Screens(BTreeMap<String, String>);

    impl Terminal for Screens {
        fn open(&self, _: &str, _: &str) -> Result<String, String> {
            unreachable!()
        }
        fn run(&self, _: &str, _: &str) -> Result<(), String> {
            unreachable!()
        }
        fn agent(&self, _: &str) -> Result<Option<dash_core::tasks::Seen>, String> {
            unreachable!()
        }
        fn sessions(&self) -> Result<usize, String> {
            unreachable!()
        }
        fn read(&self, pane: &str) -> Result<String, String> {
            self.0
                .get(pane)
                .cloned()
                .ok_or(format!("{pane}: no such pane"))
        }
        fn send(&self, _: &str, _: &str) -> Result<(), String> {
            unreachable!("the watcher types nothing")
        }
        fn key(&self, _: &str, _: &str) -> Result<(), String> {
            unreachable!("the watcher types nothing")
        }
        fn close(&self, _: &str) -> Result<(), String> {
            unreachable!()
        }
    }

    /// Task pane-fail-detect, done-when 3: a claimed pane printing « Error 143 » lands one pane failure on the pass that
    /// reads it, due at once; the next pass, 30 s on, lands none and wakes none; a working pane and one that does not read
    /// are never read as failed.
    #[dash_test]
    fn a_failing_pane_wakes_once_within_a_minute() {
        use crate::watch;
        let screens = Screens(BTreeMap::from([
            ("w1:p1".to_string(), "vibe run\nError 143\n\n".to_string()),
            ("w2:p1".to_string(), "Error 143\n  Running…\n".to_string()),
        ]));
        let tasks = BTreeMap::from([
            ("w1:p1".to_string(), "rust-x".to_string()),
            ("w2:p1".to_string(), "rust-y".to_string()),
            ("w3:p1".to_string(), "rust-z".to_string()),
        ]);
        let mut log = dash_core::MemoryLog::default();
        assert_eq!(
            pass(&mut log, &screens, &tasks, &[], 0),
            1,
            "w2 runs a tool, w3 does not read"
        );
        let items = watch::load(&log).expect("rows");
        let due = watch::due(&items, 0);
        assert_eq!(due.len(), 1, "due on the pass that landed it");
        assert_eq!(due[0].key, "pane w1:p1 rust-x error-143");
        let keys = vec![due[0].key.clone()];
        let mut told = items.clone();
        watch::told(&mut told, &keys, 0);
        watch::save(&mut log, &items, &told).expect("saved");
        assert_eq!(
            pass(&mut log, &screens, &tasks, &[], 30_000),
            0,
            "once by its key"
        );
        let busy = [Status {
            pane: "w1:p1".into(),
            state: "working".into(),
            kind: None,
            at: 0,
            session: None,
        }];
        let signatures = Signatures::tracked();
        assert!(
            failures(&screens, &tasks, &busy, signatures, 0).is_empty(),
            "a pane herdr reads working is not read"
        );
        assert!(
            watch::due(&watch::load(&log).expect("rows"), 30_000).is_empty(),
            "woken once"
        );
        // the wake records no outcome: 15 minutes on, and not before, it is one ask on his inbox
        let rows = watch::load(&log).expect("rows");
        let asks = BTreeMap::new();
        let early = watch::unrecorded(&rows, &asks, watch::OUTCOME_MS - 1);
        assert!(early.is_empty(), "not 15 minutes yet");
        let raised = watch::unrecorded(&rows, &asks, watch::OUTCOME_MS);
        let ids: Vec<&str> = raised.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["wake pane w1:p1 rust-x error-143"], "escalated");
    }

    /// Done-when 1: each signature of the tracked list matches its incident's screen, and a working screen matches none.
    #[dash_test]
    fn each_signature_matches_its_incident() {
        let env = "$ claude --task x\nenv: node: No such file or directory\n";
        assert!(failed("env-no-such-file")(&read(env)), "{:?}", read(env));
        let limit = "⎿ You've hit your limit · resets 5pm\n  What do you want to do?\n  ❯ 1. Stop and wait for limit to reset\n    2. Upgrade your plan\n";
        assert!(failed("limit-dialog")(&read(limit)));
        let killed = "⏺ vibe run\nError 143\n\n\n";
        assert!(failed("error-143")(&read(killed)));
        let prompt = "✻ Worked for 3m\n\nnicolasha@mac task-x % \n\n";
        assert!(failed("bare-prompt")(&read(prompt)));
        assert!(failed("bare-prompt")(&read("bash-5.2$ ")));
        let footer = "│ > \n╰──╯\n  Context left until auto-compact: 12%\n";
        assert_eq!(read(footer), Read::Quiet, "a percentage is no prompt");
        let dotenv = "dotenv: .env: No such file or directory\n│ > \n";
        assert_eq!(read(dotenv), Read::Quiet, "env's own line only");
    }

    /// Done-when 4: a done line above a running tool call is working, not done; nor is a failure line above it.
    #[dash_test]
    fn a_done_line_above_a_running_tool_call_is_working() {
        let screen = "TASK_DONE\ntask: x\noutcome: met\n\n⏺ Bash(cargo nextest run -p dash-comments)\n  ⎿  Running…\n\n✻ Baking… (12s · esc to interrupt)\n";
        assert_eq!(read(screen), Read::Working);
        assert_eq!(
            read("TASK_DONE\ntask: x\n│ > \n"),
            Read::Done,
            "nothing runs below it"
        );
        let below = "Error 143\n⏺ Bash(make)\n  ⎿  Running…\n";
        assert_eq!(read(below), Read::Working, "a run after the failure line");
        let ended = "⏺ Bash(./run)\n  Running... tests\nError 143\nnicolasha@mac task-x % \n";
        assert!(
            failed("error-143")(&read(ended)),
            "an old working word above the failure: {:?}",
            read(ended)
        );
        let finished = "TASK_DONE\ntask: x\nnicolasha@mac task-x % \n";
        assert_eq!(
            read(finished),
            Read::Done,
            "a finished run's shell is no failure"
        );
    }

    /// Only the foot counts: a failure scrolled above the last lines is history.
    #[dash_test]
    fn a_failure_above_the_foot_is_history() {
        let mut screen = String::from("Error 143\n");
        screen.extend((0..20).map(|n| format!("line {n}\n")));
        assert_eq!(read(&screen), Read::Quiet);
    }

    /// The list refuses a row a reader cannot match.
    #[dash_test]
    fn a_row_with_no_matcher_is_refused() {
        let bad = r#"{"fails": [{"name": "x", "incident": "y"}]}"#;
        assert!(Signatures::parse(bad).is_err());
    }
}
