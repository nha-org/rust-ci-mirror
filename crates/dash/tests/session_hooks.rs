//! `dash hook session-state|statusline|gh-meter-path` as .claude/settings.json runs them: Claude Code's JSON on
//! stdin, in a scratch folder with no live session's variables (task rust-hooks-rest-port; the shell and Python hooks they
//! replace had these cases as test/fixtures/hooks).
use dash_meter::dash_test;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const LIVE: [&str; 12] = [
    "CLAUDE_SUPERVISOR",
    "CLAUDE_ENV_FILE",
    "CLAUDE_CONFIG_DIR",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_ENTRYPOINT",
    "DASH_SESSION",
    "HERDR_DELEGATE",
    "HERDR_PANE_ID",
    "HERDR_TAB_ID",
    "HERDR_WORKSPACE_ID",
    "ORCA_AGENT_HOOK_PORT",
    "CLAUDE_WEEKLY_LIMIT_DIR",
];

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("session-hooks-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("scratch");
    d
}

fn hook(args: &[&str], dir: &Path, env: &[(&str, &str)], input: &str) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_dash"));
    c.arg("hook")
        .args(args)
        .current_dir(dir)
        .env("HOME", dir.join("home"));
    for v in LIVE {
        c.env_remove(v);
    }
    let mut child = c
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("dash");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("write");
    child.wait_with_output().expect("wait")
}

fn read(p: PathBuf) -> String {
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// Done-when 3 of rust-hooks-rest-port, session-state.sh and stop.sh: unsupervised it does nothing; supervised, a start
/// records the session and its thread line, an approval is told once at a tool call, never to a subagent, and the Stop
/// (`idle`) writes its running background tasks for bin/claude-supervise.
#[dash_test]
fn session_state_tells_the_supervisor_what_it_cannot_see() {
    let d = scratch("state");
    let dir = d.display().to_string();
    let start = r#"{"session_id":"s1","hook_event_name":"SessionStart","source":"startup"}"#;
    let out = hook(
        &["session-state", "start"],
        &d,
        &[("CLAUDE_PROJECT_DIR", &dir)],
        start,
    );
    assert!(out.status.success() && out.stdout.is_empty() && !d.join(".claude").exists());
    let sup = [
        ("CLAUDE_PROJECT_DIR", dir.as_str()),
        ("CLAUDE_SUPERVISOR", "sup"),
    ];
    let out = hook(&["session-state", "start"], &d, &sup, start);
    assert!(out.status.success());
    let run = d.join(".claude/run");
    assert_eq!(read(run.join("sup.session")), "s1\n");
    assert_eq!(read(run.join("sup.state")), "idle\n");
    assert!(
        String::from_utf8_lossy(&out.stdout)
            .starts_with("This session runs under an old supervisor (pid sup)")
    );
    let thread = read(d.join("home/.claude/run/threads.jsonl"));
    assert!(thread.starts_with(r#"{"ev":"start","at":"20"#), "{thread}");
    assert!(thread.ends_with(&format!(r#""source":"startup","thread":"pane:none","id":"s1","account":"default","workspace":"","tab":"","pane":"","cwd":"{dir}"}}
"#)), "{thread}");
    // an approval it does not run: told once at a tool call, never to a subagent
    fs::write(run.join("approved"), "abcdef0123456789\n").expect("approved");
    fs::write(run.join("sup.supervisor"), "0000\n").expect("loaded");
    assert!(
        hook(&["session-state", "tool"], &d, &sup, r#"{"agent_id":"a1"}"#)
            .stdout
            .is_empty()
    );
    let told = hook(&["session-state", "tool"], &d, &sup, "{}");
    let said: serde_json::Value = serde_json::from_slice(&told.stdout).expect("JSON");
    assert_eq!(said["hookSpecificOutput"]["hookEventName"], "PostToolUse");
    assert!(
        said["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap_or("")
            .starts_with("Nicolas approved abcdef012345,")
    );
    assert!(
        hook(&["session-state", "tool"], &d, &sup, "{}")
            .stdout
            .is_empty(),
        "once per approval"
    );
    assert_eq!(
        read(run.join("sup.state")),
        "busy\n",
        "a tool call is a turn running"
    );
    // the Stop
    let stop = r#"{"session_id":"s1","background_tasks":[{"type":"subagent","id":"a9","status":"running","description":"review"},{"type":"shell","id":"b1","status":"running","command":"wake-wait"}]}"#;
    assert!(
        hook(&["session-state", "idle"], &d, &sup, stop)
            .status
            .success()
    );
    assert_eq!(read(run.join("sup.agents")), "subagent a9 (review)\n");
    assert_eq!(read(run.join("sup.jobs")), "shell b1 wake-wait\n");
    assert_eq!(read(run.join("sup.state")), "idle\n");
    let log = fs::read_dir(&run)
        .expect("run")
        .flatten()
        .find(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("session-state.")
        })
        .map(|e| read(e.path()))
        .expect("the day's log");
    assert!(
        log.contains(" sup idle state=idle approved=abcdef012345 asked=n agents=1\n"),
        "{log}"
    );
    let _ = fs::remove_dir_all(&d);
}

/// Done-when 3, statusline-usage.sh: a status line without limits writes nothing, one with them is kept per session.
#[dash_test]
fn the_status_line_keeps_each_session_s_rate_limits() {
    let d = scratch("statusline");
    let weekly = d.join("weekly").display().to_string();
    let env = [("CLAUDE_WEEKLY_LIMIT_DIR", weekly.as_str())];
    let out = hook(
        &["statusline"],
        &d,
        &env,
        r#"{"session_id":"s1","model":{"id":"x"}}"#,
    );
    assert!(out.status.success() && out.stdout.is_empty() && !d.join("weekly").exists());
    let input = r#"{"session_id":"s1","rate_limits":{"five_hour":{"used_percentage":10},"seven_day":{"used_percentage":20}}}"#;
    assert!(hook(&["statusline"], &d, &env, input).status.success());
    let kept = read(d.join("weekly/s1.json"));
    assert_eq!(kept, format!("{{\"config_dir\":\"\",{}", &input[1..]));
    let _ = fs::remove_dir_all(&d);
}

/// Done-when 3, gh-meter-path.sh: the gh shim goes first on the session's PATH, once.
#[dash_test]
fn the_gh_shim_goes_first_on_the_session_s_path_once() {
    let d = scratch("ghmeter");
    let shim = d.join("private/dashboard/priv/gh-meter");
    fs::create_dir_all(&shim).expect("shim");
    fs::write(shim.join("gh"), "#!/bin/sh\n").expect("gh");
    Command::new("chmod")
        .args(["+x"])
        .arg(shim.join("gh"))
        .status()
        .expect("chmod");
    let (dir, file) = (d.display().to_string(), d.join("env").display().to_string());
    let env = [
        ("CLAUDE_PROJECT_DIR", dir.as_str()),
        ("CLAUDE_ENV_FILE", file.as_str()),
    ];
    for _ in 0..2 {
        assert!(hook(&["gh-meter-path"], &d, &env, "{}").status.success());
    }
    assert_eq!(
        read(d.join("env")),
        format!("export PATH=\"{}:$PATH\"\n", shim.display())
    );
    let _ = fs::remove_dir_all(&d);
}
