//! Core's Terminal on herdr, through its CLI (`HERDR_BIN`, else `herdr` on PATH). Each call is bounded: a herdr that hangs
//! never holds a run.
use dash_core::tasks::{Seen, Terminal};
use dash_process as process;
use serde_json::Value;
use std::time::Duration;

pub mod say;

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

/// Core's Terminal on herdr-core, as a client of its CLI's JSON (`hc_bin`; rfc-rust-unified 20f): never a link to its
/// unpublished crate. Each call is bounded like herdr's.
pub struct Hc {
    bin: String,
    limit: Duration,
}

impl Hc {
    pub fn from_env() -> Self {
        Self {
            bin: hc_bin(),
            limit: Duration::from_secs(15),
        }
    }

    fn call(&self, args: &[&str]) -> Result<Value, String> {
        self.call_for(args, self.limit)
    }

    // An hc that never started (not there, not executable, or refused by safehouse) says NO_HC with the system's reason,
    // never a bare spawn error: the page tells him how to install it (/code-review: a stat passed a file that cannot run).
    fn call_for(&self, args: &[&str], limit: Duration) -> Result<Value, String> {
        let args = [&["--json"], args].concat();
        let (status, text, reason) =
            process::answer(&self.bin, &args, None, &[], limit).map_err(|no| match no.started {
                false => format!(
                    "{NO_HC} {}: {}",
                    self.bin,
                    no.why.rsplit(": ").next().unwrap_or_default()
                ),
                true => no.why,
            })?;
        if !status.success() {
            let what = format!("{} {}", self.bin, args.join(" "));
            return Err(format!(
                "{what}: {status}: {} {}",
                reason.trim(),
                text.trim()
            ));
        }
        serde_json::from_str(&text).map_err(|e| format!("hc's answer: {e}: {text}"))
    }
}

/// hc's binary: `HC_BIN`, else ~/.local/bin/hc, where he installed it (2026-11-10). Never a bare `hc`: launchd's PATH, dash
/// serve's, holds no ~/.local/bin, and the live /terminal answered 500 « hc --json read x: No such file or directory ».
pub fn hc_bin() -> String {
    std::env::var("HC_BIN").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/.local/bin/hc")
    })
}

/// How a failure starts when hc never started: no hc to run.
pub const NO_HC: &str = "no hc at";

/// A failure that says hc never started: none installed where dash looks, or one it may not run.
pub fn no_hc(e: &str) -> bool {
    e.starts_with(NO_HC)
}

/// A failure of hc's that names a terminal it does not know.
pub fn unknown(e: &str) -> bool {
    e.contains(r#""kind":"not_found""#)
}

/// The terminal `hc run --bg` started.
fn hc_address(answer: &Value) -> Result<String, String> {
    let address = answer["address"].as_str().map(String::from);
    address.ok_or(format!("no address in: {answer}"))
}

/// A terminal's agent in `hc get`'s answer. Our terminals are a shell running the launch line, so an agent is whatever program is
/// in front of that shell, starting or at work, as herdr lists one before its first status. Its session is that program's own
/// address and pid, which no other run in that terminal shares: hc keeps no Claude session id. Its state is what it said through
/// OSC 7501; one that said nothing yet reads as busy. With no foreground read (a remote machine), a status said is the sign.
fn hc_seen(answer: &Value) -> Option<Seen> {
    if answer["state"] != "live" {
        return None;
    }
    let front = &answer["foreground"]["pid"];
    let word = answer["status"]["state"].as_str();
    let program = match front.as_i64() {
        Some(pid) if Some(pid) == answer["pid"].as_i64() => return None,
        Some(pid) => pid,
        None => {
            word?;
            answer["pid"].as_i64().unwrap_or_default()
        }
    };
    Some(Seen {
        idle: matches!(word, Some("idle" | "done")),
        asking: word == Some("blocked"),
        session: Some(format!("{}/{program}", answer["address"].as_str()?)),
    })
}

/// The Claude sessions in `hc ls`'s answer: its `agent` is the program's own name, else its command's.
fn hc_sessions(answer: &Value) -> Result<usize, String> {
    let terminals = answer["terminals"].as_array();
    let terminals = terminals.ok_or(format!("no terminals in: {answer}"))?;
    Ok(terminals.iter().filter(|t| t["agent"] == "claude").count())
}

/// The screen in `hc read`'s answer.
fn hc_text(answer: &Value) -> Result<String, String> {
    let text = answer["text"].as_str().map(String::from);
    text.ok_or(format!("no text in: {answer}"))
}

impl Terminal for Hc {
    // A terminal of its own, with no parent: it outlives whoever opened it, as a herdr workspace does. Herdr.app shows it as its
    // own space and tab under `label`.
    fn open(&self, folder: &str, label: &str) -> Result<String, String> {
        let space = format!("herdr.space={label}");
        let tab = format!("herdr.tab={label}");
        let args = [
            "run",
            "--bg",
            "--no-parent",
            "--cwd",
            folder,
            "--label",
            &space,
            "--label",
            &tab,
        ];
        hc_address(&self.call(&args)?)
    }

    fn run(&self, pane: &str, line: &str) -> Result<(), String> {
        self.send(pane, line)
    }

    // A terminal hc no longer has is no agent, as herdr's list leaves it out.
    fn agent(&self, pane: &str) -> Result<Option<Seen>, String> {
        match self.call(&["get", pane]) {
            Ok(answer) => Ok(hc_seen(&answer)),
            Err(e) if unknown(&e) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn sessions(&self) -> Result<usize, String> {
        hc_sessions(&self.call(&["ls"])?)
    }

    fn read(&self, pane: &str) -> Result<String, String> {
        hc_text(&self.call(&["read", pane])?)
    }

    // hc pastes it whole and presses Enter itself, so the caller's Enter after it lands on an empty prompt. --force types into a
    // program that asks, as herdr's send-text does. Its limit holds hc's wait for a person's draft (60 s after his last key).
    fn send(&self, pane: &str, text: &str) -> Result<(), String> {
        let args = ["send", "--force", "--", pane, text];
        self.call_for(&args, Duration::from_secs(75)).map(drop)
    }

    // hc's key names take herdr's in any case: Enter, Down, Escape.
    fn key(&self, pane: &str, key: &str) -> Result<(), String> {
        self.call(&["keys", pane, key]).map(drop)
    }

    fn close(&self, pane: &str) -> Result<(), String> {
        self.call(&["kill", pane]).map(drop)
    }
}

/// The terminal `DASH_TERMINAL` names: `hc` for herdr-core, else herdr.
pub fn from_env() -> Box<dyn Terminal> {
    match std::env::var("DASH_TERMINAL").as_deref() {
        Ok("hc") => Box::new(Hc::from_env()),
        _ => Box::new(Herdr::from_env()),
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

    // hc's answers as its CLI prints them piped (herdr-core ce81486: cli.rs, cli/output.rs `short`, terminals.rs `Public`), each
    // keyed by its subcommand; the stand-in logs every call it gets.
    const HC: &str = r#"#!/bin/sh
echo "$@" >> "$(dirname "$0")/calls"
case "$2" in
run) echo '{"address":"t7","state":"live","agent":"zsh","cwd":"/w/a","where":"a"}' ;;
get) case "$3" in
  t7) echo '{"address":"t7","name":null,"labels":{"herdr.space":"rust claude"},"argv":["/bin/zsh"],"cwd":"/w/a","keep":false,"state":"live","pid":42,"exit_code":null,"signal":null,"reason":null,"created_ms":1,"finished_ms":null,"screen":"exact","cols":200,"rows":120,"identity":"ghostty","status":{"state":"blocked","kind":"question","app":"claude"},"status_ms":2,"foreground":{"pid":51,"name":"2.1.322","argv":["claude"],"cwd":"/w/a"}}' ;;
  t8) echo '{"address":"t8","argv":["/bin/zsh"],"cwd":"/w/a","state":"live","pid":43,"status":null,"status_ms":null,"foreground":{"pid":52,"name":"make","argv":["make","claude"],"cwd":"/w/a"}}' ;;
  t4) echo '{"address":"t4","argv":["/bin/zsh"],"cwd":"/w/a","state":"live","pid":44,"status":{"state":"done","app":"claude"},"foreground":{"pid":44,"name":"zsh","argv":["-zsh"],"cwd":"/w/a"}}' ;;
  m/t2) echo '{"address":"m/t2","argv":["claude"],"cwd":"/w/a","state":"live","pid":9,"status":{"state":"idle"},"foreground":null}' ;;
  m/t3) echo '{"address":"m/t3","argv":["/bin/sh"],"cwd":"/w/a","state":"live","pid":8,"status":null,"foreground":null}' ;;
  t6) echo '{"address":"t6","argv":["claude"],"cwd":"/w/a","state":"finished","exit_code":0,"reason":"exited","status":null}' ;;
  t5) echo '{"error":{"kind":"unavailable","message":"core did not start within 5 s","hint":""}}' >&2; exit 5 ;;
  *) echo '{"error":{"kind":"not_found","message":"no terminal \"'"$3"'\"","hint":"see `hc ls --all`"}}' >&2; exit 3 ;;
  esac ;;
ls) echo '{"terminals":[{"address":"t1","state":"live","status":"working","agent":"claude","cwd":"/w/a"},{"address":"t2","state":"live","agent":"zsh","cwd":"/w/b"},{"address":"t3","state":"live","status":"idle","agent":"claude","cwd":"/w/c"}],"hidden_finished":4}' ;;
read) printf '%s\n' '{"text":"$ echo hc-live-42\nhc-live-42\n$","complete":true,"how":"screen"}' ;;
send) echo '{"status":"idle","turn":3,"started":true}' ;;
keys) echo '{"status":"blocked","turn":3}' ;;
kill) echo '{"address":"t7","state":"finished","reason":"killed","status":null}' ;;
*) echo '{"error":{"kind":"usage","message":"unknown","hint":""}}' >&2; exit 2 ;;
esac
"#;

    /// The stand-in, written into `dir`: its path.
    fn stand_in(dir: &TestDir) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let bin = dir.0.join("hc");
        std::fs::write(&bin, HC).expect("stand-in");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("mode");
        bin
    }

    // nextest runs each test in a process of its own, so its environment is this test's alone.
    #[test]
    fn dash_terminal_picks_hc_else_herdr() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("terminal-pick");
        let bin = stand_in(&dir);
        unsafe {
            std::env::set_var("HC_BIN", &bin);
            std::env::set_var("HERDR_BIN", &bin);
            std::env::set_var("DASH_TERMINAL", "hc");
        }
        assert_eq!(from_env().open("/w/a", "x").expect("hc"), "t7");
        unsafe { std::env::remove_var("DASH_TERMINAL") };
        assert!(
            from_env().open("/w/a", "x").is_err(),
            "herdr's call, which the stand-in refuses"
        );
        let calls = std::fs::read_to_string(dir.0.join("calls")).expect("calls");
        let want = [
            "--json run --bg --no-parent --cwd /w/a --label herdr.space=x --label herdr.tab=x",
            "workspace create --cwd /w/a --label x --no-focus",
        ];
        assert_eq!(calls.lines().collect::<Vec<_>>(), want);
    }

    /// Done-when 1 of rust-terminal-no-hc: hc is `HC_BIN`, else ~/.local/bin/hc, never a bare `hc` looked up on PATH; a binary
    /// that is not there, or cannot run, fails as NO_HC.
    #[test]
    fn hc_is_hc_bin_else_local_bin_never_a_bare_name() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("terminal-hc-bin");
        let bin = stand_in(&dir);
        unsafe {
            std::env::remove_var("HC_BIN");
            std::env::set_var("HOME", &dir.0);
            // a bare `hc` on PATH would answer: it is never looked for
            std::env::set_var("PATH", &dir.0);
        }
        let home = dir.0.display();
        assert_eq!(hc_bin(), format!("{home}/.local/bin/hc"));
        let none = Hc::from_env().read("t7").expect_err("no hc there");
        let want = format!("{NO_HC} {home}/.local/bin/hc: No such file or directory (os error 2)");
        assert_eq!(none, want);
        std::fs::create_dir_all(dir.0.join(".local/bin")).expect("folder");
        std::fs::rename(&bin, dir.0.join(".local/bin/hc")).expect("installed");
        assert!(
            Hc::from_env()
                .read("t7")
                .expect("read")
                .contains("hc-live-42")
        );
        let plain = dir.0.join("plain");
        std::fs::write(&plain, "not a program").expect("file");
        unsafe { std::env::set_var("HC_BIN", &plain) };
        let held = Hc::from_env()
            .read("t7")
            .expect_err("there, not executable");
        assert!(no_hc(&held), "{held}");
        unsafe { std::env::set_var("HC_BIN", dir.0.join("elsewhere")) };
        let set = Hc::from_env().read("t7").expect_err("HC_BIN first");
        assert!(
            set.starts_with(&format!("{NO_HC} {home}/elsewhere: ")),
            "{set}"
        );
    }

    #[test]
    fn hc_s_recorded_answers_read_as_panes_and_states() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("terminal-hc");
        let bin = stand_in(&dir);
        let hc = Hc {
            bin: bin.display().to_string(),
            limit: Duration::from_secs(10),
        };
        assert_eq!(hc.open("/w/a", "rust claude").expect("open"), "t7");
        let seen = |idle, asking, session: &str| {
            Some(Seen {
                idle,
                asking,
                session: Some(session.into()),
            })
        };
        assert_eq!(hc.agent("t7").expect("get"), seen(false, true, "t7/51"));
        let starting = hc.agent("t8").expect("get");
        assert_eq!(
            starting,
            seen(false, false, "t8/52"),
            "busy before its first status"
        );
        assert_eq!(
            hc.agent("t4").expect("get"),
            None,
            "its shell is back in front"
        );
        assert_eq!(hc.agent("m/t2").expect("get"), seen(true, false, "m/t2/9"));
        assert_eq!(
            hc.agent("m/t3").expect("get"),
            None,
            "no front read, no status said"
        );
        assert_eq!(hc.agent("t6").expect("get"), None, "its program ended");
        assert_eq!(hc.agent("t9").expect("gone"), None, "a terminal hc dropped");
        assert_eq!(hc.sessions().expect("ls"), 2);
        assert!(hc.read("t7").expect("read").contains("\nhc-live-42\n"));
        hc.run("t7", "echo hc-live-$((40+2))").expect("run");
        hc.send("t7", "--goal ship it").expect("send");
        hc.key("t7", "Enter").expect("keys");
        hc.close("t7").expect("kill");
        let calls = std::fs::read_to_string(dir.0.join("calls")).expect("calls");
        let want = [
            "--json run --bg --no-parent --cwd /w/a --label herdr.space=rust claude --label herdr.tab=rust claude",
            "--json get t7",
            "--json get t8",
            "--json get t4",
            "--json get m/t2",
            "--json get m/t3",
            "--json get t6",
            "--json get t9",
            "--json ls",
            "--json read t7",
            "--json send --force -- t7 echo hc-live-$((40+2))",
            "--json send --force -- t7 --goal ship it",
            "--json keys t7 Enter",
            "--json kill t7",
        ];
        assert_eq!(calls.lines().collect::<Vec<_>>(), want);
        eprintln!("hc calls, each answered in hc's JSON:\n{calls}");
        let down = hc.agent("t5").expect_err("any other failure stays one");
        assert!(down.contains("core did not start"), "{down}");
        assert!(hc_address(&serde_json::json!({})).is_err());
        assert!(hc_sessions(&serde_json::json!({"error": {}})).is_err());
        assert!(hc_text(&serde_json::json!({})).is_err());
    }

    /// Reads process::OUTPUT_LIMIT, 16 MB, and holds it as bytes and text: 66 to 69 MB above its start on the Mac.
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

    /// Reads process::OUTPUT_LIMIT, 16 MB, and holds it as bytes and text: 66 to 69 MB above its start on the Mac.
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
