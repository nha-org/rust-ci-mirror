//! Claude Code's hooks on the send tools, each a .claude/hooks/*.py once, its selftest the tests here (task
//! rust-hooks-rest-port; @nha, 2026-11-10: « all these python scripts are also very brittle. we want to move all logic to
//! rust and have it well tested. »):
//! 1. **`dash hook form-fill-guard`** (PreToolUse, claude-in-chrome): a form_input that writes text is refused, since it
//!    overwrites what he typed (@nha, 2026-10-23, on the Mistral form: « you seems to have discarded my pre filled answers.
//!    Mix please. »). Text goes through .claude/skills/browser-apply/fill-merge.js; a checkbox's bool still passes.
//! 2. **`dash hook linkedin-budget [--post]`** (Pre and PostToolUse, the LinkedIn MCP): each call asks the dashboard's
//!    LinkedIn gate (Dashboard.Linkedin) whether its page loads fit, and fails closed when it does not answer (Nicolas,
//!    2026-09-27: "we DEFINITELY dont want LinkedIn BAN ! ... rate limit + day limit"). A task calls only what its brief
//!    names. `--post`: a failed call that reads as a limit starts the day-long cooldown, in the gate or, app down, its file.
//! 3. **`dash hook outbound-log`** (PostToolUse, the sends): each message sent through an MCP is one line of the model's
//!    outbound log through dash's bridge to the older app (`Host::outbound`, dash_bridge::Dash::outbound), which keeps a line
//!    it surely could not log in the main checkout's private/live/pending/ for the app's next check (Nicolas, 2026-10-03: "Messages on LinkedIn mcp etc should have a date. Email same.").
//!
//! A hook that cannot read its input says nothing, as the .py did; a refusing hook that fails refuses.
use regex::Regex;
use serde_json::{Value, json};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// `dash hook <name> [<args>]` for the three hooks here: Claude Code's JSON on stdin, a deny on stdout.
pub fn run(name: &str, args: &[&str], mut out: File) {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let Ok(event) = serde_json::from_str::<Value>(&input) else {
        // the LinkedIn gate fails closed, as the .py's crash did under `|| exit 2`; a line to log is never dropped silently
        match (name, args) {
            ("linkedin-budget", []) => {
                let why = "LinkedIn gate: the hook read no JSON, so the call is refused.";
                let _ = writeln!(out, "{}", crate::deny(why));
            }
            ("outbound-log", ["--line"]) => {
                eprintln!("outbound line not logged: it is no JSON: {input}");
                std::process::exit(1);
            }
            _ => {}
        }
        return;
    };
    let here = || {
        crate::project_dir()
            .map(PathBuf::from)
            .or_else(|| event["cwd"].as_str().map(PathBuf::from))
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default()
    };
    let said = match (name, args) {
        ("form-fill-guard", []) => form_fill(&event),
        ("linkedin-budget", []) => {
            let task = (crate::host().task_brief)(&here());
            linkedin_pre(&event, task.as_ref(), &crate::host().linkedin_gate)
        }
        ("linkedin-budget", ["--post"]) => {
            linkedin_post(
                &event,
                &crate::host().linkedin_gate,
                &cooldown_file(),
                now_secs(),
            );
            None
        }
        ("outbound-log", []) => {
            let at = (crate::host().local_at)(now_secs() as i64);
            if let Some(line) = outbound_entry(&event, &at)
                && let Err(why) = (crate::host().outbound)(&line)
            {
                eprintln!("{why}");
            }
            None
        }
        // a line a script already made (freework_apply.py's application): logged or kept the same way
        ("outbound-log", ["--line"]) => {
            if let Err(why) = (crate::host().outbound)(&event.to_string()) {
                eprintln!("{why}");
                if !why.starts_with("dashboard log unconfirmed")
                    && !why.starts_with("outbound send kept")
                {
                    std::process::exit(1);
                }
            }
            None
        }
        _ => {
            eprintln!("dash hook {name}: no such mode {args:?}");
            std::process::exit(2);
        }
    };
    if let Some(said) = said {
        let _ = writeln!(out, "{said}");
    }
}

/// The host's clock, as every other hook reads it (/simplify of PR 1480).
fn now_secs() -> f64 {
    (crate::host().now_ms)() as f64 / 1000.0
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// Python's truth of a JSON value: null, false, 0, "" and empty containers are false.
pub(crate) fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

// ---------------------------------------------------------------------------------------------------- form-fill-guard --

pub(crate) const FILL_WHY: &str = "Form fill refused: form_input overwrites what he typed. Run .claude/skills/browser-apply/fill-merge.js through javascript_tool (the file's text, then fillMerge([{field, value}, ...])): it keeps his answers and shows ours beside them. Skill: .claude/skills/browser-apply/SKILL.md";
const FORM_INPUT: &str = "mcp__claude-in-chrome__form_input";

/// The call sets a field's text: form_input alone or inside a batch, with a value that is no bool (a checkbox's true or
/// false). An input of another shape reads as a text, so it is refused, as the .py's crash was.
fn overwrites(tool: &str, args: &Value) -> bool {
    match tool {
        FORM_INPUT => !args["value"].is_boolean(),
        "mcp__claude-in-chrome__browser_batch" => {
            args["actions"].as_array().is_some_and(|actions| {
                actions.iter().any(|a| {
                    matches!(a["name"].as_str(), Some("form_input" | FORM_INPUT))
                        && overwrites(FORM_INPUT, &a["input"])
                })
            })
        }
        _ => false,
    }
}

pub(crate) fn form_fill(event: &Value) -> Option<String> {
    let tool = event["tool_name"].as_str().unwrap_or_default();
    overwrites(tool, &event["tool_input"]).then(|| crate::deny(FILL_WHY))
}

// ---------------------------------------------------------------------------------------------------- linkedin-budget --

const LINKEDIN: &str = "mcp__mcp-server-linkedin__";
/// one call may load at most this many pages (@nha, 2026-09-27: « we really cannot afford to get kicked out of linked in »)
const MAX_PAGES: i64 = 3;
/// LinkedIn meters page loads: one search loads max_pages pages (3 by default), a profile one per extra section
const PAGED: [&str; 3] = ["search_jobs", "get_saved_jobs", "search_posts"];
const SECTIONED: [&str; 3] = [
    "get_person_profile",
    "get_my_profile",
    "get_company_profile",
];
const COOL_FOR: f64 = 86400.0;
const LIMITED: [&str; 5] = [
    "rate limit",
    "too many requests",
    "http 429",
    "status 429",
    "unusual activity",
];

/// The gate's JSON answer to `op`, or None when the app does not answer (`Host::linkedin_gate`, dash's bridge to it).
type Ask<'a> = &'a dyn Fn(&str, &Value) -> Option<Value>;

fn cooldown_file() -> PathBuf {
    std::env::var_os("LINKEDIN_COOLDOWN_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".claude/.linkedin-mcp/cooldown.json"))
}

/// Page loads one call makes, from its arguments; a page count that reads as no number is an error (the .py's ValueError).
fn pages(name: &str, args: &Value) -> Result<i64, String> {
    if name == "get_conversation" {
        // by thread id, the thread alone; by username, the profile, the inbox and the thread
        return Ok(if truthy(&args["thread_id"]) { 1 } else { 3 });
    }
    if PAGED.contains(&name) {
        let n = &args["max_pages"];
        if !truthy(n) {
            return Ok(3);
        }
        return match n {
            Value::Number(n) => n.as_f64().map(|f| f.trunc() as i64),
            Value::String(s) => s.trim().parse().ok(),
            Value::Bool(_) => Some(1),
            _ => None,
        }
        .ok_or_else(|| format!("ValueError: max_pages {n} is no number"));
    }
    if SECTIONED.contains(&name) {
        let sections = match &args["sections"] {
            Value::String(s) => s.clone(),
            v if !truthy(v) => String::new(),
            v => v.to_string(),
        };
        return Ok(1 + sections.split(',').filter(|s| !s.trim().is_empty()).count() as i64);
    }
    Ok(1)
}

fn budget_refusal(name: &str, args: &Value, ask: Ask) -> Result<Option<String>, String> {
    let n = pages(name, args)?;
    if n > MAX_PAGES {
        return Ok(Some(format!(
            "LinkedIn budget: this call would load {n} pages, the limit is {MAX_PAGES} per call. Ask for fewer pages or sections."
        )));
    }
    let Some(answer) = ask("check", &json!({"tool": name, "pages": n})) else {
        return Ok(Some("LinkedIn gate: the dashboard does not answer, so no LinkedIn call runs (it fails closed). Start the app; do not work around it.".into()));
    };
    if answer["allow"] == json!(true) {
        return Ok(None);
    }
    Ok(Some(match answer["reason"].as_str() {
        Some(r) if !r.is_empty() => r.to_string(),
        _ => format!("LinkedIn gate refused: {answer}"),
    }))
}

/// A task session reaches LinkedIn only for a call its brief names: a task re-tried get_my_profile four times while the
/// account was limited (@nha, 2026-10-20: « we should never hit a rate limit. This is putting us in danger to get linked in
/// bannned »). `task`: the session's task and its brief, "" with none (`Host::task_brief`); none for a session of no task. No
/// brief, no call.
fn unbriefed(task: Option<&(String, String)>, name: &str) -> Option<String> {
    let (id, brief) = task?;
    // the name as a whole word: get_inbox and mcp__mcp-server-linkedin__get_inbox name it, get_inbox_all does not
    let named = brief.match_indices(name).any(|(at, _)| {
        let before = brief[..at].chars().next_back();
        let after = brief[at + name.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_alphanumeric())
            && !after.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
    });
    (!named).then(|| {
        format!(
            "LinkedIn: task {id}'s brief does not name {name}, so a task does not call it: tests run on fixtures, never the live account. If the task needs it, say so in your report."
        )
    })
}

/// PreToolUse: the deny, or None. `task`: the task of the session's folder (CLAUDE_PROJECT_DIR, fixed for it: the event's
/// cwd follows each cd out of the worktree) and its brief.
pub(crate) fn linkedin_pre(
    event: &Value,
    task: Option<&(String, String)>,
    ask: Ask,
) -> Option<String> {
    let tool = event["tool_name"].as_str().unwrap_or_default();
    let name = tool.strip_prefix(LINKEDIN).unwrap_or(tool);
    // local, never touches LinkedIn
    if name == "close_session" {
        return None;
    }
    let reason = match unbriefed(task, name) {
        Some(r) => Some(r),
        None => budget_refusal(name, &event["tool_input"], ask).unwrap_or_else(|e| {
            Some(format!(
                "LinkedIn gate: the hook failed ({e}), so the call is refused."
            ))
        }),
    };
    reason.map(|r| crate::deny(&r))
}

/// An object anywhere in `v` flagged isError or is_error true: an MCP error may come back as a result, not a failure event.
fn flagged_error(v: &Value) -> bool {
    match v {
        Value::Object(o) => {
            ["isError", "is_error"]
                .iter()
                .any(|k| o.get(*k) == Some(&json!(true)))
                || o.values().any(flagged_error)
        }
        Value::Array(a) => a.iter().any(flagged_error),
        _ => false,
    }
}

/// PostToolUse(Failure): a failed call that reads as a limit starts the day-long cooldown, in the gate or, app down, the file
/// the gate reads at each decision. Only a failure: a job ad asking for « API rate limiting » is a success that says « rate
/// limit », and the gate's own refusal quotes it too (/code-review).
pub(crate) fn linkedin_post(event: &Value, ask: Ask, cooldown: &Path, now: f64) {
    let said = json!([event["error"], event["tool_response"], event["tool_output"]]);
    if event["hook_event_name"] != "PostToolUseFailure" && !flagged_error(&said) {
        return;
    }
    let out = said.to_string().to_lowercase();
    if out.contains("linkedin gate") || out.contains("linkedin cooldown") {
        return;
    }
    let Some(hit) = LIMITED.iter().find(|w| out.contains(*w)) else {
        return;
    };
    let tool = event["tool_name"].as_str().unwrap_or_default();
    let why = format!(
        "{} answered « {hit} »",
        tool.strip_prefix(LINKEDIN).unwrap_or(tool)
    );
    if ask("limited", &json!({"why": why})).is_some_and(|a| a["ok"] == json!(true)) {
        return;
    }
    let line = json!({"until": now + COOL_FOR, "since": now, "why": why.chars().take(300).collect::<String>()});
    let tmp = cooldown.with_extension("json.tmp");
    let wrote =
        std::fs::write(&tmp, line.to_string()).and_then(|()| std::fs::rename(&tmp, cooldown));
    if let Err(e) = wrote {
        eprintln!(
            "linkedin-budget: the cooldown was not written to {}: {e}",
            cooldown.display()
        );
    }
}

// ------------------------------------------------------------------------------------------------------- outbound-log --

/// The tools that send something to a person, and their channel.
const SENDS: [(&str, &str); 8] = [
    ("mcp__mcp-server-linkedin__send_message", "linkedin"),
    ("mcp__mcp-server-linkedin__reply_in_thread", "linkedin"),
    ("mcp__mcp-server-linkedin__connect_with_person", "linkedin"),
    ("mcp__mcp-server-linkedin__message_job_poster", "linkedin"),
    ("mcp__mcp-server-linkedin__easy_apply", "form"),
    ("mcp__claude_ai_Gmail__send_message", "email"),
    ("mcp__claude_ai_Gmail__reply", "email"),
    ("mcp__claude_ai_Gmail__forward", "email"),
];
const TO: [&str; 13] = [
    "to",
    "recipient",
    "recipients",
    "profile_url",
    "linkedin_username",
    "username",
    "person",
    "thread_id",
    "conversation_id",
    "job_url",
    "job_id",
    "message_id",
    "thread",
];
const TEXT: [&str; 5] = ["message", "body", "text", "note", "content"];

/// The outbound line of one call, `{date, at, channel, tool, to, text}`, or None when it sent nothing. `at` is the local
/// time, `YYYY-MM-DDTHH:MM:SS`.
pub(crate) fn outbound_entry(data: &Value, at: &str) -> Option<String> {
    let tool = data["tool_name"].as_str()?;
    let channel = SENDS.iter().find(|(t, _)| *t == tool)?.1;
    let resp: String = data["tool_response"]
        .to_string()
        .chars()
        .take(2000)
        .collect();
    static ERROR: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#""(isError|is_error)":\s*true"#).expect("regex"));
    static UNSENT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"\\*"sent\\*"\s*:\s*false"#).expect("regex"));
    static CONNECTED: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"\\*"status\\*"\s*:\s*\\*"(connected|accepted)\\*""#).expect("regex")
    });
    // The LinkedIn fork answers sent false when the thread, read after the send, does not show the message, or nothing was
    // submitted: never logged as a send. Ruby's reply, never delivered, was logged twice (11-03, 11-05, linkedin-send-confirms).
    if ERROR.is_match(&resp) || UNSENT.is_match(&resp) {
        return None;
    }
    // a connect's answer has no sent: only these two statuses mean a request left or an invite was accepted (/code-review of PR 1173)
    if tool.ends_with("__connect_with_person") && !CONNECTED.is_match(&resp) {
        return None;
    }
    let input = &data["tool_input"];
    let pick = |keys: &[&str]| keys.iter().map(|k| &input[*k]).find(|v| truthy(v)).cloned();
    let to = match pick(&TO) {
        Some(Value::String(s)) => json!(s),
        Some(v) => json!(v.to_string()),
        // the .py's json.dumps(None): readers take `to` as a string
        None => json!("null"),
    };
    let text = match pick(&TEXT) {
        Some(Value::String(s)) => s,
        Some(v) => v.to_string(),
        None => String::new(),
    };
    let line = json!({"date": at.get(..10).unwrap_or(at), "at": at, "channel": channel,
                      "tool": tool.rsplit("__").next().unwrap_or(tool), "to": to,
                      "text": text.chars().take(300).collect::<String>()});
    Some(line.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;
    use std::cell::RefCell;

    fn reason(said: Option<String>) -> String {
        let said: Value = serde_json::from_str(&said.expect("a deny")).expect("JSON");
        assert_eq!(said["hookSpecificOutput"]["permissionDecision"], "deny");
        said["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap_or_default()
            .into()
    }

    /// form-fill-guard.py's selftest, case for case.
    #[dash_test]
    fn form_fill_refuses_a_text_and_lets_a_checkbox_and_a_merge_through() {
        let fi = FORM_INPUT;
        let batch = "mcp__claude-in-chrome__browser_batch";
        let call =
            |tool: &str, input: Value| form_fill(&json!({"tool_name": tool, "tool_input": input}));
        assert!(
            reason(call(fi, json!({"ref": "r1", "value": "Kafka, 8 years"})))
                .contains("fill-merge.js")
        );
        assert!(call(fi, json!({"ref": "r1", "value": 650})).is_some());
        assert!(call(fi, json!({"ref": "r1", "value": true})).is_none());
        assert!(call(fi, json!({"ref": "r1", "value": "false"})).is_some());
        assert!(call(fi, json!("not a dict")).is_some());
        assert!(
            call(
                batch,
                json!({"actions": [{"name": "form_input", "input": "x"}]})
            )
            .is_some()
        );
        let mixed = json!({"actions": [{"name": "navigate", "input": {"url": "x"}}, {"name": "form_input", "input": {"value": "a"}}]});
        assert!(call(batch, mixed).is_some());
        assert!(
            call(
                batch,
                json!({"actions": [{"name": "computer", "input": {"action": "left_click"}}]})
            )
            .is_none()
        );
        assert!(
            call(
                "mcp__claude-in-chrome__javascript_tool",
                json!({"text": "fillMerge([])"})
            )
            .is_none()
        );
    }

    /// linkedin-budget.py's selftest: pages, the per-call cap, the gate failing closed, a crash refusing.
    #[dash_test]
    fn linkedin_counts_pages_and_fails_closed() {
        assert_eq!(pages("search_jobs", &json!({})), Ok(3));
        assert_eq!(pages("search_jobs", &json!({"max_pages": 1})), Ok(1));
        assert_eq!(
            pages(
                "get_person_profile",
                &json!({"sections": "experience, education"})
            ),
            Ok(3)
        );
        assert_eq!(pages("get_inbox", &json!({"limit": 20})), Ok(1));
        assert_eq!(pages("get_conversation", &json!({})), Ok(3));
        assert_eq!(
            pages("get_conversation", &json!({"thread_id": "2-abc"})),
            Ok(1)
        );
        let call = |tool: &str, input: Value, ask: Ask| {
            linkedin_pre(
                &json!({"tool_name": LINKEDIN.to_string() + tool, "tool_input": input}),
                None,
                ask,
            )
        };
        let allow = |_: &str, _: &Value| Some(json!({"allow": true}));
        let down = |_: &str, _: &Value| None;
        assert!(reason(call("search_jobs", json!({"max_pages": 5}), &allow)).contains("per call"));
        assert!(reason(call("get_inbox", json!({}), &down)).contains("fails closed"));
        assert!(call("get_inbox", json!({}), &allow).is_none());
        assert!(call("close_session", json!({}), &down).is_none());
        let no = |_: &str, _: &Value| {
            Some(json!({"allow": false, "reason": "LinkedIn day limit reached"}))
        };
        assert_eq!(
            reason(call("get_inbox", json!({}), &no)),
            "LinkedIn day limit reached"
        );
        assert!(
            reason(call("search_jobs", json!({"max_pages": "all"}), &allow))
                .contains("hook failed")
        );
    }

    #[dash_test]
    fn linkedin_post_cools_down_on_a_failed_limit_only() {
        let dir = std::env::temp_dir().join(format!("linkedin-post-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let file = dir.join("cooldown.json");
        let asked = RefCell::new(Vec::new());
        let down = |op: &str, _: &Value| {
            asked.borrow_mut().push(op.to_string());
            None
        };
        let ad = json!({"hook_event_name": "PostToolUse", "tool_name": LINKEDIN.to_string() + "get_job_details",
                        "tool_response": "Experience with API rate limiting"});
        linkedin_post(&ad, &down, &file, 100.0);
        assert!(!file.exists(), "a success that says rate limit");
        let refused = json!({"hook_event_name": "PostToolUseFailure", "tool_name": LINKEDIN.to_string() + "get_inbox",
                             "error": "LinkedIn cooldown until 2026-10-21: get_my_profile answered « rate limit »"});
        linkedin_post(&refused, &down, &file, 100.0);
        assert!(!file.exists(), "the gate's own refusal");
        let limited = json!({"hook_event_name": "PostToolUseFailure", "tool_name": LINKEDIN.to_string() + "get_my_profile",
                             "error": "Rate limit detected. Wait 300 seconds"});
        linkedin_post(&limited, &down, &file, 100.0);
        let kept: Value =
            serde_json::from_str(&std::fs::read_to_string(&file).expect("the cooldown"))
                .expect("JSON");
        assert_eq!(kept["why"], "get_my_profile answered « rate limit »");
        assert_eq!(kept["until"], json!(100.0 + COOL_FOR));
        assert_eq!(*asked.borrow(), ["limited"]);
        // an error flagged in a result, not a failure event, counts; the gate taking it writes no file
        std::fs::remove_file(&file).expect("rm");
        let flagged = json!({"hook_event_name": "PostToolUse", "tool_name": LINKEDIN.to_string() + "get_inbox",
                             "tool_response": {"isError": true, "content": "HTTP 429"}});
        linkedin_post(
            &flagged,
            &|_: &str, _: &Value| Some(json!({"ok": true})),
            &file,
            100.0,
        );
        assert!(!file.exists());
        linkedin_post(&flagged, &down, &file, 100.0);
        assert!(file.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[dash_test]
    fn linkedin_a_task_calls_only_what_its_brief_names() {
        let task = |brief: &str| Some(("li-check".to_string(), brief.to_string()));
        // a session of no task: only the gate decides
        assert!(unbriefed(None, "get_my_profile").is_none());
        let inbox = task("Read the inbox once with get_inbox.");
        assert!(unbriefed(inbox.as_ref(), "get_inbox").is_none());
        assert!(
            unbriefed(inbox.as_ref(), "get_my_profile")
                .is_some_and(|r| r.contains("task li-check's brief does not name get_my_profile"))
        );
        // no brief written: no call
        assert!(unbriefed(task("").as_ref(), "get_inbox").is_some());
        let full = task("call mcp__mcp-server-linkedin__get_inbox once");
        assert!(unbriefed(full.as_ref(), "get_inbox").is_none());
        assert!(unbriefed(full.as_ref(), "get_inbox_all").is_some());
    }

    /// outbound-log.py's selftest: one line per send, none for a read, an error or a send the fork could not confirm.
    #[dash_test]
    fn outbound_log_writes_a_line_per_confirmed_send_only() {
        let at = "2026-10-03T18:00:00";
        let entry = |tool: &str, input: Value, response: Value| {
            outbound_entry(
                &json!({"tool_name": tool, "tool_input": input, "tool_response": response}),
                at,
            )
            .map(|l| serde_json::from_str::<Value>(&l).expect("JSON"))
        };
        let li = |t: &str| format!("mcp__mcp-server-linkedin__{t}");
        assert_eq!(
            entry(
                &li("send_message"),
                json!({"linkedin_username": "jdoe", "message": "Bonjour"}),
                json!({"ok": true})
            ),
            Some(
                json!({"date": "2026-10-03", "at": at, "channel": "linkedin", "tool": "send_message", "to": "jdoe",
                        "text": "Bonjour"})
            )
        );
        assert_eq!(entry(&li("get_inbox"), json!({}), Value::Null), None);
        // no recipient key: the string "null", as the .py's json.dumps(None) wrote it
        let no_to = entry(
            &li("easy_apply"),
            json!({"message": "x"}),
            json!({"ok": true}),
        );
        assert_eq!(no_to.map(|e| e["to"].clone()), Some(json!("null")));
        let gmail = "mcp__claude_ai_Gmail__send_message";
        assert_eq!(
            entry(
                gmail,
                json!({"to": ["a@b.c"], "body": "x"}),
                json!({"isError": true})
            ),
            None
        );
        assert_eq!(
            entry(gmail, json!({"to": ["a@b.c"], "body": "x"}), json!("sent"))
                .map(|e| e["to"].clone()),
            Some(json!("[\"a@b.c\"]"))
        );
        let wrapped = |r: Value| json!([{"type": "text", "text": r.to_string()}]);
        for (label, response, logged) in [
            (
                "send_unconfirmed",
                wrapped(json!({"status": "send_unconfirmed", "sent": false, "retry_safe": false})),
                false,
            ),
            (
                "not_sent",
                wrapped(json!({"status": "not_sent", "sent": false, "retry_safe": false})),
                false,
            ),
            (
                "not_sent, spaced JSON",
                json!([{"type": "text", "text": "{\"status\": \"not_sent\", \"sent\": false}"}]),
                false,
            ),
            (
                "not_sent, plain object",
                json!({"status": "not_sent", "sent": false}),
                false,
            ),
            (
                "sent",
                wrapped(json!({"status": "sent", "sent": true, "retry_safe": false})),
                true,
            ),
        ] {
            let got = entry(
                &li("reply_in_thread"),
                json!({"thread_id": "t", "message": "x"}),
                response,
            );
            assert_eq!(got.is_some(), logged, "{label}");
        }
        let connect = |status: &str| {
            entry(
                &li("connect_with_person"),
                json!({"linkedin_username": "j"}),
                wrapped(json!({"status": status, "note_sent": false})),
            )
            .is_some()
        };
        assert!(
            connect("connected")
                && connect("accepted")
                && !connect("send_failed")
                && !connect("pending")
        );
    }
}
