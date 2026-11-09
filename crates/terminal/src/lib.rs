//! Core's Terminal on herdr, through its CLI (`HERDR_BIN`, else `herdr` on PATH). Each call is bounded: a herdr that hangs
//! never holds a run.
use dash_core::tasks::{Seen, Terminal};
use dash_process as process;
use serde_json::Value;
use std::time::Duration;

pub struct Herdr {
    bin: String,
    limit: Duration,
}

impl Herdr {
    pub fn from_env() -> Self {
        Self {
            bin: std::env::var("HERDR_BIN").unwrap_or_else(|_| "herdr".into()),
            limit: Duration::from_secs(15),
        }
    }

    fn call(&self, args: &[&str]) -> Result<String, String> {
        process::run(&self.bin, args, None, self.limit)
    }
}

fn result(text: &str) -> Result<Value, String> {
    let json: Value = serde_json::from_str(text).map_err(|e| format!("herdr's answer: {e}"))?;
    Ok(json["result"].clone())
}

/// The pane in `workspace create`'s answer.
fn root_pane(text: &str) -> Result<String, String> {
    let pane = result(text)?["root_pane"]["pane_id"]
        .as_str()
        .map(String::from);
    pane.ok_or(format!("no root pane in: {text}"))
}

/// That pane's agent in `agent list`'s answer. A status herdr adds later reads as busy: it stops and starts nothing.
fn agent_in(text: &str, pane: &str) -> Result<Option<Seen>, String> {
    let result = result(text)?;
    let agents = result["agents"].as_array();
    let agents = agents.ok_or(format!("no agents in: {text}"))?;
    let agent = agents.iter().find(|a| a["pane_id"] == pane);
    Ok(agent.map(|a| Seen {
        idle: matches!(a["agent_status"].as_str(), Some("idle" | "done")),
        asking: a["agent_status"] == "blocked",
        session: a["agent_session"]["value"].as_str().map(String::from),
    }))
}

/// The Claude sessions `agent list` shows, of any project and however idle: what his seats count (Admission.claude).
fn sessions_in(text: &str) -> Result<usize, String> {
    let result = result(text)?;
    let agents = result["agents"].as_array();
    let agents = agents.ok_or(format!("no agents in: {text}"))?;
    Ok(agents.iter().filter(|a| a["agent"] == "claude").count())
}

/// The ids of the Claude sessions `agent list` shows, `None` for one with no id yet: what a login's removal checks, so none is
/// left out.
fn session_ids_in(text: &str) -> Result<Vec<Option<String>>, String> {
    let result = result(text)?;
    let agents = result["agents"].as_array();
    let agents = agents.ok_or(format!("no agents in: {text}"))?;
    let claude = agents.iter().filter(|a| a["agent"] == "claude");
    Ok(claude
        .map(|a| a["agent_session"]["value"].as_str().map(String::from))
        .collect())
}

/// One agent herdr shows, for /agents: what it works on as its tab's title says, its state, and where it runs.
#[derive(Clone, Debug, PartialEq)]
pub struct Live {
    pub title: String,
    pub status: String,
    pub cwd: String,
}

/// Every agent `agent list` shows, of any kind and project, in its order.
fn live_in(text: &str) -> Result<Vec<Live>, String> {
    let result = result(text)?;
    let agents = result["agents"].as_array();
    let agents = agents.ok_or(format!("no agents in: {text}"))?;
    let said = |a: &Value, key: &str| a[key].as_str().unwrap_or_default().to_string();
    Ok(agents
        .iter()
        .map(|a| {
            let title = said(a, "terminal_title_stripped");
            Live {
                title: if title.is_empty() {
                    said(a, "agent")
                } else {
                    title
                },
                status: said(a, "agent_status"),
                cwd: said(a, "cwd"),
            }
        })
        .collect())
}

impl Herdr {
    /// The agents open, of any project: what each works on.
    pub fn live(&self) -> Result<Vec<Live>, String> {
        live_in(&self.call(&["agent", "list"])?)
    }

    /// The Claude sessions open, of any project: their ids.
    pub fn claude_sessions(&self) -> Result<Vec<Option<String>>, String> {
        session_ids_in(&self.call(&["agent", "list"])?)
    }
}

impl Terminal for Herdr {
    // A workspace of its own, never a tab of the caller's: session.sh's own fallback, and no row's tab count moves. Its label
    // matches none of Dashboard.Watch.Tabs's, so that watcher leaves it as one of his.
    fn open(&self, folder: &str, label: &str) -> Result<String, String> {
        let args = [
            "workspace",
            "create",
            "--cwd",
            folder,
            "--label",
            label,
            "--no-focus",
        ];
        root_pane(&self.call(&args)?)
    }

    fn run(&self, pane: &str, line: &str) -> Result<(), String> {
        self.call(&["pane", "run", pane, line]).map(drop)
    }

    fn agent(&self, pane: &str) -> Result<Option<Seen>, String> {
        agent_in(&self.call(&["agent", "list"])?, pane)
    }

    fn sessions(&self) -> Result<usize, String> {
        sessions_in(&self.call(&["agent", "list"])?)
    }

    fn read(&self, pane: &str) -> Result<String, String> {
        self.call(&["pane", "read", pane])
    }

    // One bracketed paste: typed as keys, Claude Code kept only the tail of a long message (rfc-dashboard R16).
    fn send(&self, pane: &str, text: &str) -> Result<(), String> {
        let paste = format!("\u{1b}[200~{text}\u{1b}[201~");
        self.call(&["pane", "send-text", pane, &paste]).map(drop)
    }

    fn key(&self, pane: &str, key: &str) -> Result<(), String> {
        self.call(&["pane", "send-keys", pane, key]).map(drop)
    }

    fn close(&self, pane: &str) -> Result<(), String> {
        self.call(&["pane", "close", pane]).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::memory::TestBudget;

    #[test]
    fn herdr_s_answers_read_as_panes_and_states() {
        let _budget = TestBudget::new();
        let created = r#"{"result":{"root_pane":{"pane_id":"w9:pA"}}}"#;
        assert_eq!(root_pane(created).expect("pane"), "w9:pA");
        assert!(root_pane(r#"{"result":{}}"#).is_err());
        let list = r#"{"result":{"agents":[
          {"pane_id":"a","agent_status":"working","agent_session":{"value":"s1"}},
          {"pane_id":"b","agent_status":"done"},
          {"pane_id":"c","agent_status":"idle"},
          {"pane_id":"d","agent_status":"blocked"},
          {"pane_id":"e","agent_status":"something new"}]}}"#;
        let seen = |pane| agent_in(list, pane).expect("list").expect("agent");
        let idle = ["a", "b", "c", "d", "e"].map(|p| seen(p).idle);
        assert_eq!(idle, [false, true, true, false, false]);
        let asking = ["a", "b", "c", "d", "e"].map(|p| seen(p).asking);
        assert_eq!(asking, [false, false, false, true, false]);
        assert_eq!(seen("a").session.as_deref(), Some("s1"));
        assert_eq!(agent_in(list, "gone").expect("list"), None);
        assert!(agent_in(r#"{"error":"no"}"#, "a").is_err());
        let seats =
            r#"{"result":{"agents":[{"agent":"claude"},{"agent":"vibe"},{"agent":"claude"}]}}"#;
        assert_eq!(sessions_in(seats).expect("list"), 2);
        let ids = r#"{"result":{"agents":[{"agent":"claude","agent_session":{"value":"s1"}},
          {"agent":"vibe","agent_session":{"value":"v"}},{"agent":"claude"}]}}"#;
        assert_eq!(
            session_ids_in(ids).expect("list"),
            [Some("s1".to_string()), None],
            "a Claude session with no id yet is kept"
        );
        let live = r#"{"result":{"agents":[
          {"agent":"claude","agent_status":"working","cwd":"/w/a","terminal_title_stripped":"Rust design components"},
          {"agent":"vibe","agent_status":"idle","cwd":"/w/b","terminal_title_stripped":""}]}}"#;
        let one = |title: &str, status: &str, cwd: &str| Live {
            title: title.into(),
            status: status.into(),
            cwd: cwd.into(),
        };
        assert_eq!(
            live_in(live).expect("list"),
            [
                one("Rust design components", "working", "/w/a"),
                one("vibe", "idle", "/w/b")
            ],
            "every agent, its title or else its kind"
        );
        assert!(live_in(r#"{"error":"no"}"#).is_err());
    }

    #[test]
    fn process_run_returns_err_on_tool_output_past_its_limit() {
        let _budget = TestBudget::of(100.0);
        let ten = Duration::from_secs(10);
        let head = |bytes: u64| {
            let program = format!("head -c {bytes} /dev/zero");
            process::run("/bin/sh", &["-c", &program], None, ten)
        };
        let at_it = head(process::OUTPUT_LIMIT).expect("at the limit");
        assert_eq!(at_it.len() as u64, process::OUTPUT_LIMIT);
        drop(at_it);
        let past = head(process::OUTPUT_LIMIT + 1).expect_err("past it");
        assert!(past.contains("its output is past"), "{past}");
        // It ends the tool too, at once: neither of these would finish before the call's limit.
        let begin = std::time::Instant::now();
        let endless = process::run("yes", &[], None, ten).expect_err("past it");
        assert!(endless.contains("its output is past"), "{endless}");
        let then_silent = format!(
            "head -c {} /dev/zero; exec sleep 30",
            process::OUTPUT_LIMIT + 1
        );
        let silent =
            process::run("/bin/sh", &["-c", &then_silent], None, ten).expect_err("past it");
        assert!(silent.contains("its output is past"), "{silent}");
        assert!(begin.elapsed() < ten / 2, "{:?}", begin.elapsed());
    }

    #[test]
    fn process_run_counts_the_text_it_holds_not_the_bytes_it_read() {
        let _budget = TestBudget::of(100.0);
        let ten = Duration::from_secs(10);
        let stray = |bytes: u64| {
            let program = format!("head -c {bytes} /dev/zero | LC_ALL=C tr '\\0' '\\377'");
            process::run("/bin/sh", &["-c", &program], None, ten)
        };
        assert_eq!(stray(1).expect("a stray byte"), "\u{FFFD}");
        let past = stray(process::OUTPUT_LIMIT).expect_err("three times the limit as text");
        assert!(past.contains("its output is past"), "{past}");
    }

    #[test]
    fn a_tool_s_long_stderr_is_no_failure_and_a_failure_s_reason_keeps_its_end() {
        let _budget = TestBudget::new();
        let ten = Duration::from_secs(10);
        // The reason is computed, since a failure's message also holds the command line.
        let noisy = |end: &str| {
            let program = format!(
                "head -c {} /dev/zero >&2 || exit 9; echo fatal: $((40+2)) >&2; echo ok; {end}",
                process::OUTPUT_LIMIT + 1
            );
            process::run("/bin/sh", &["-c", &program], None, ten)
        };
        assert_eq!(noisy("exit 0").expect("not a failure"), "ok\n");
        let failed = noisy("exit 3").expect_err("its exit");
        assert!(failed.contains("exit status: 3"), "{failed}");
        assert!(failed.contains("fatal: 42"), "{failed}");
        assert!(failed.len() < 128 * 1024, "{}", failed.len());
    }

    // core takes no test budget, so its one tool caller is tested here (crates/gate).
    #[test]
    fn a_call_reads_past_a_pipe_s_size_says_a_failure_and_ends_a_hang() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("terminal-call");
        let big = dir.0.join("big");
        std::fs::write(&big, "x".repeat(200 * 1024)).expect("file");
        let ten = Duration::from_secs(10);
        let text =
            process::run("/bin/cat", &[big.to_str().expect("path")], None, ten).expect("all of it");
        assert_eq!(text.len(), 200 * 1024);
        assert!(
            process::run("/bin/cat", &["/no/such/file"], None, ten)
                .is_err_and(|e| e.contains("exit"))
        );
        let short = Duration::from_millis(100);
        let hung = process::run("/bin/sleep", &["30"], None, short);
        assert!(hung.is_err_and(|e| e.contains("no answer")));
        let left_behind = process::run("/bin/sh", &["-c", "sleep 2 & exit 0"], None, short);
        assert!(left_behind.is_err_and(|e| e.contains("never closed")));
        let here = process::run("/bin/pwd", &[], Some(&dir.0), ten).expect("pwd");
        let real = std::fs::canonicalize(&dir.0).expect("real");
        assert_eq!(here.trim(), real.to_str().expect("path"));
    }
}
