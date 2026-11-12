//! Section 20b phase 3: a second agent kind, wired here so orch keeps its one Agent and one Policy. The pick reads every kind's
//! limits, a start is given only an account of its own task's kind, and a run starts its kind's agent.
use dash_agent::ClaudeCode;
use dash_agent::codex::{self, Codex};
use dash_core::tasks::{
    Agent, Limit, Outcome, Pick, Policy, Queue, Queued, Screen, Snapshot, Step, Verdict,
};
use dash_policy::Defaults;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long the picker keeps a Codex reading: each read starts `codex app-server`, about a second, and a pass comes each minute.
const CODEX_FRESH: Duration = Duration::from_secs(5 * 60);

pub struct Agents {
    pub claude: ClaudeCode,
    pub codex: Codex,
    /// The last Codex reading and when: a failed one is kept as none, so a broken Codex costs one wait per 5 minutes.
    codex_read: Mutex<Option<(Instant, Vec<Limit>)>>,
    /// The run store's folder, whose key rows' limits the snapshot holds too (rust-key-agent).
    data: PathBuf,
}

impl Agents {
    pub fn new(claude: ClaudeCode, codex: Codex, data: &Path) -> Self {
        let codex_read = Mutex::default();
        let data = data.to_path_buf();
        Self {
            claude,
            codex,
            codex_read,
            data,
        }
    }

    /// Codex's limits, read again once the last reading is 5 minutes old. A read that fails is no Codex spend known.
    pub fn codex_limits(&self) -> Vec<Limit> {
        let mut read = self.codex_read.lock().expect("the reading");
        match &*read {
            Some((at, limits)) if at.elapsed() < CODEX_FRESH => limits.clone(),
            _ => {
                let limits = self.codex.limits().unwrap_or_default();
                *read = Some((Instant::now(), limits.clone()));
                limits
            }
        }
    }
}

/// The picker's agent: every kind's limits, the key rows' last, the rest Claude's, as `next` asks it nothing else. A Codex or key
/// read that fails never hides Claude's.
impl Agent for Agents {
    fn screen(&self, shown: &str) -> Screen {
        self.claude.screen(shown)
    }
    fn outcome(&self, folder: &str, session: &str) -> Result<Option<Outcome>, String> {
        self.claude.outcome(folder, session)
    }
    fn stop_words(&self) -> String {
        self.claude.stop_words()
    }
    fn limits(&self) -> Result<Vec<Limit>, String> {
        let mut limits = self.claude.limits()?;
        limits.extend(self.codex_limits());
        limits.extend(crate::keys::key_limits(&self.data));
        Ok(limits)
    }
    fn reachable(&self) -> bool {
        self.claude.reachable()
    }
    fn limit_menu(&self, shown: &str) -> bool {
        self.claude.limit_menu(shown)
    }
}

/// The default policy on each task's own kind: a Codex limit is no room for a Claude task, nor the reverse, so a task whose kind
/// is spent leaves its turn to the next one of the other kind. `is_codex` reads a task's kind, never while no seat is free.
/// ponytail: one kind read per task looked at, until one starts; keep them with the queue if a pass's time shows it.
pub struct ByKind<F> {
    pub is_codex: F,
}

impl<F: Fn(&str) -> bool> Policy for ByKind<F> {
    fn pick(&self, snapshot: &Snapshot) -> Pick {
        // No task or no seat: its words, no kind read. With no limit known, Defaults starts.
        let free = Snapshot {
            limits: vec![],
            ..snapshot.clone()
        };
        let free = Defaults.pick(&free);
        if !matches!(free, Pick::Start { .. }) {
            return free;
        }
        let mut first_wait = None;
        for task in &snapshot.queue.tasks {
            let codex = (self.is_codex)(&task.id);
            let limits = snapshot.limits.iter();
            let limits = limits.filter(|l| (l.account == codex::ACCOUNT) == codex);
            let alone = Snapshot {
                queue: Queue {
                    seats: snapshot.queue.seats,
                    seats_why: snapshot.queue.seats_why.clone(),
                    // a Codex task runs on its one model: its route names no Claude pool (RFC 20h)
                    tasks: vec![match codex {
                        true => Queued {
                            route: Some(vec![]),
                            ..task.clone()
                        },
                        false => task.clone(),
                    }],
                },
                sessions: snapshot.sessions,
                limits: limits.cloned().collect(),
                free_percent: snapshot.free_percent,
                now: snapshot.now,
            };
            match Defaults.pick(&alone) {
                start @ Pick::Start { .. } => return start,
                wait => {
                    first_wait.get_or_insert(wait);
                }
            }
        }
        first_wait.unwrap_or(free)
    }
    fn after(&self, verdict: &Verdict, nudges: u32, past_limit: bool) -> Step {
        Defaults.after(verdict, nudges, past_limit)
    }
    fn retry(&self, attempts: u32) -> bool {
        Defaults.retry(attempts)
    }
    fn holds(&self, task: &Queued, free_percent: Option<u8>) -> Option<String> {
        Defaults.holds(task, free_percent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_core::tasks::{Queued, plan};
    use dash_meter::dash_test;
    use serde_json::Value;
    use std::path::{Path, PathBuf};

    /// Codex's limit as app-server answered on his login, 2026-11-08, its use set to `used`.
    fn recorded(used: u32) -> Limit {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../agent/fixtures/codex/rate-limits.json");
        let text = std::fs::read_to_string(path).expect("fixture");
        let mut reply: Value = serde_json::from_str(&text).expect("json");
        reply["result"]["rateLimitsByLimitId"]["codex"]["primary"]["usedPercent"] = used.into();
        codex::limit_in(&reply["result"], 1_794_160_000).expect("a limit")
    }

    fn claude(account: &str, week: f64, week_resets: u64) -> Limit {
        let account = account.into();
        let five_hours = 0.0;
        Limit {
            key: false,
            account,
            five_hours,
            five_hours_resets: 0,
            week,
            week_resets,
            models: vec![],
        }
    }

    /// The plan's pick, a task's kind read from its name; `read` counts the kind reads.
    fn picks(tasks: &[&str], sessions: usize, limits: Vec<Limit>, read: &Mutex<u32>) -> Pick {
        let queued = |id: &&str| Queued {
            held: None,
            id: id.to_string(),
            title: String::new(),
            reopened_ms: None,
            waits: vec![],
            layer: None,
            builds_nothing: false,
            rank: Default::default(),
            free: None,
            // a `hard-` name is hard work (RFC 20h), on its default route
            demand: match id.contains("hard-") {
                true => dash_core::tasks::Demand::Hard,
                false => Default::default(),
            },
            route: None,
            ..Default::default()
        };
        let queue = Queue {
            seats: 5,
            seats_why: String::new(),
            tasks: tasks.iter().map(queued).collect(),
        };
        let policy = ByKind {
            is_codex: |id: &str| {
                *read.lock().expect("count") += 1;
                id.starts_with("codex-")
            },
        };
        plan(&policy, queue, &[], sessions, limits, None).next
    }

    fn pick(task: &str, limits: Vec<Limit>) -> Pick {
        picks(&[task], 0, limits, &Mutex::new(0))
    }

    fn start(id: &str, account: Option<&str>) -> Pick {
        let (id, account) = (id.into(), account.map(String::from));
        Pick::Start {
            id,
            account,
            pool: None,
        }
    }

    fn waits(pick: &Pick, words: &str) {
        assert!(
            matches!(pick, Pick::Wait(w) if w.contains(words)),
            "{pick:?}"
        );
    }

    #[dash_test]
    fn codex_s_recorded_limit_reaches_the_pick_and_only_a_codex_task_spends_it() {
        // Codex's 30 days reset after this Claude week: by spend order alone, a Claude task would start on Codex.
        let soon = claude("second", 10.0, 1_796_000_000);
        let all = || vec![recorded(0), soon.clone()];
        assert_eq!(pick("codex-t", all()), start("codex-t", Some("codex")));
        assert_eq!(pick("t", all()), start("t", Some("second")));
        let spent = claude("second", 100.0, 1_796_000_000);
        waits(
            &pick("t", vec![recorded(0), spent.clone()]),
            "every account",
        );
        let codex_room = pick("codex-t", vec![recorded(0), spent.clone()]);
        assert_eq!(codex_room, start("codex-t", Some("codex")));
        waits(&pick("codex-t", vec![recorded(100), soon]), "every account");
        let unread = pick("codex-t", vec![spent]);
        assert_eq!(
            unread,
            start("codex-t", None),
            "no Codex reading, nothing says stop"
        );
    }

    /// /code-review of rust-pools-fable, finding 8: a hard Codex task names no Claude pool, starting or waiting: its wait never
    /// blames Fable, and a hard Claude task beside it takes Fable.
    #[dash_test]
    fn a_hard_codex_task_spends_no_claude_pool() {
        let soon = claude("second", 10.0, 1_796_000_000);
        let started = pick("codex-hard-t", vec![recorded(0), soon.clone()]);
        assert_eq!(started, start("codex-hard-t", Some("codex")));
        let spent = pick("codex-hard-t", vec![recorded(100), soon.clone()]);
        waits(&spent, "every account");
        assert!(!format!("{spent:?}").contains("fable"), "{spent:?}");
        let claude_hard = pick("hard-t", vec![recorded(0), soon]);
        assert!(
            matches!(&claude_hard, Pick::Start { account: Some(a), pool: Some(p), .. } if a == "second" && p.pool == "fable"),
            "{claude_hard:?}"
        );
    }

    #[dash_test]
    fn a_task_whose_kind_is_spent_leaves_its_turn_to_the_other_kind() {
        let spent = claude("second", 100.0, 1_796_000_000);
        // ranked by id when all else is equal: the Claude task first
        let queue = ["claude-t", "codex-t"];
        let read = Mutex::new(0);
        let other = picks(&queue, 0, vec![recorded(0), spent.clone()], &read);
        assert_eq!(other, start("codex-t", Some("codex")));
        let both = picks(&queue, 0, vec![recorded(100), spent], &Mutex::new(0));
        waits(&both, "claude-t waits: every account");
        let reads = *read.lock().expect("count");
        let full = Mutex::new(0);
        waits(
            &picks(&queue, 5, vec![recorded(0)], &full),
            "5 sessions open of 5 allowed",
        );
        assert_eq!(
            (reads, *full.lock().expect("count")),
            (2, 0),
            "no kind read without a seat"
        );
    }

    #[dash_test]
    fn a_codex_read_that_fails_never_hides_claude_s() {
        let dir = dash_meter::TestDir::new("kinds-limits");
        let reading = serde_json::json!({"config_dir": "/h/.claude", "rate_limits": {
            "seven_day": {"used_percentage": 20, "resets_at": u64::MAX}}});
        std::fs::write(dir.0.join("s.json"), reading.to_string()).expect("reading");
        let claude = ClaudeCode {
            projects: PathBuf::new(),
            limits: dir.0.clone(),
        };
        let bin = dir.0.join("no-codex").display().to_string();
        let sessions = PathBuf::new();
        let data = dash_meter::TestDir::new("kinds-limits-data");
        let agents = Agents::new(claude, Codex { bin, sessions }, &data.0);
        let limits = agents.limits().expect("Claude's");
        assert_eq!(limits.len(), 1);
        let read = (limits[0].account.as_str(), limits[0].week);
        assert_eq!(read, ("default", 20.0));
        let kept = agents.codex_read.lock().expect("reading").clone();
        assert!(
            matches!(kept, Some((_, ref l)) if l.is_empty()),
            "kept as none for 5 minutes"
        );
    }
}
