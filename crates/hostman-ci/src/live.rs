//! The runner kept up live (task rust-hostman-ci-wire; @nha, 2026-11-14: « If CI is the bottleneck... use hostman as I
//! asked. », « take one 8GB machine please. »): `Runner::tick` registered as a recurring job the host's queue runs each
//! minute (dash_jobs), its rows in this plugin's own database, and /status/ci-runner showing them, its alarm when Down. The
//! hostman tool is called through the host's registry, so its Cloud-Servers-only scope, its credit cap and its audit log hold;
//! GitHub is asked through the host's `forge` (his gh login), so no secret lives here.
use crate::{LABEL, MODEL, NO_RUNNER, REINSTALL_MS, Runner, State, Want};
use dash_core::{Drawing, Host, OpLog, Page, Reads, Recurring, Tick, Value, json};
use serde_json::Value as Json;

/// This plugin's database, under the host's data folder.
pub const FILE: &str = "hostman-ci.sqlite";
pub const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS ci_runner (name TEXT PRIMARY KEY, status TEXT, alarm TEXT) WITHOUT ROWID;";
/// The repository the runner registers to.
pub const REPO: &str = "nha-org/site_perso";
const EVERY_MS: i64 = 60_000;
/// A status older than this is no reading: the job is not running, and the page says so (rust/CLAUDE.md point 5).
pub const STALE_MS: i64 = 5 * EVERY_MS;

fn start(host: &'static dyn Host) -> Result<Tick, String> {
    let runners = format!("repos/{REPO}/actions/runners?per_page=100");
    let token = format!("repos/{REPO}/actions/runners/registration-token");
    let want = Want::eight_gb(REPO);
    let name = want.name.clone();
    let log = host.open(&host.data(), FILE, &[SCHEMA])?;
    let state = seed(&*log, host.now_ms())?;
    let runner = Runner::new(
        want,
        Box::new(move |asked| host.call("hostman", asked)),
        Box::new(move || online(&host.forge(&["api", &runners])?, &name)),
        Box::new(move || {
            let t = host.forge(&["api", "-X", "POST", &token, "--jq", ".token"])?;
            Some(t.trim().to_string())
                .filter(|t| !t.is_empty())
                .ok_or("GitHub gave no registration token".into())
        }),
    )
    .clocked(Box::new(move || host.now_ms()))
    .seeded(state);
    Ok(Box::new(move || runner.tick()))
}

/// The state a restart resumes, from the statuses of the last `REINSTALL_MS`: when the last reinstall was, and since when
/// the server has been on with `NO_RUNNER`, the start of the last run of such statuses. The app redeploys every few
/// minutes some days (09:27 and 09:38 on 2026-11-15): a state held in memory alone would never reach 15 minutes.
pub fn seed(log: &dyn OpLog, now_ms: i64) -> Result<State, String> {
    let mut state = State::default();
    each_status(log, now_ms - REINSTALL_MS, &mut |text, at| {
        if text.starts_with("reinstalled") {
            state.installed = Some(at);
        }
        state.since = match text.contains(NO_RUNNER) {
            true => state.since.or(Some(at)),
            false => None,
        };
    })?;
    Ok(state)
}

inventory::submit! {
    Recurring { model: MODEL, plugin: "hostman-ci", every_ms: EVERY_MS, file: FILE, schema: &[SCHEMA], start }
}

/// Whether GitHub's runner list holds the runner `name` online with `LABEL`, busy or not. The name too: the whatsnr VPS
/// runners carried the old label as a bridge, so a label alone read « up » a minute after server 9602127 was made, and its
/// missing runner was never healed (2026-11-15, 01:56 to 08:42).
pub fn online(listed: &str, name: &str) -> Result<bool, String> {
    let j: Json = serde_json::from_str(listed).map_err(|_| "GitHub's runner list is no JSON")?;
    let runners = j["runners"]
        .as_array()
        .ok_or("GitHub's runner list holds no runners")?;
    let labelled =
        |r: &Json| (r["labels"].as_array()).is_some_and(|l| l.iter().any(|l| l["name"] == LABEL));
    Ok(runners
        .iter()
        .any(|r| r["status"] == "online" && r["name"] == name && labelled(r)))
}

/// Each status the job wrote since `from_ms`, oldest first, with when, read a page at a time.
fn each_status(log: &dyn OpLog, from_ms: i64, f: &mut dyn FnMut(&str, i64)) -> Result<(), String> {
    let mut after = (from_ms << 16, String::new());
    loop {
        let ops = log.ops_after(Some((after.0, &after.1)), 500)?;
        let Some(op) = ops.last() else {
            return Ok(());
        };
        after = (op.stamp, op.writer.clone());
        for op in &ops {
            if let ("ci_runner", "status", Value::Text(t)) = (
                op.write.table.as_str(),
                op.write.field.as_str(),
                &op.write.value,
            ) {
                f(t, op.stamp >> 16);
            }
        }
    }
}

/// The last status the job wrote within `STALE_MS` of `now_ms`, read from the log's recent ops alone (a page at a time is
/// the rule: here the window is the page); none when the job wrote nothing in that time.
pub fn status(log: &dyn OpLog, now_ms: i64) -> Result<Option<(String, i64)>, String> {
    let mut last = None;
    each_status(log, now_ms - STALE_MS, &mut |t, at| {
        last = Some((t.to_string(), at))
    })?;
    Ok(last)
}

/// Whether the runner is up as the job last saw it, within `STALE_MS`: the router's room for it.
pub fn up(host: &dyn Host) -> Result<bool, String> {
    let log = host.open(&host.data(), FILE, &[SCHEMA])?;
    Ok(status(&*log, host.now_ms())?.is_some_and(|(s, _)| s == "up"))
}

/// The page's lines: its state, the status and its age, the alarm.
pub fn lines(read: Result<Option<(String, i64)>, String>, now_ms: i64) -> Vec<String> {
    let (ok, status, alarm) = match read {
        Err(e) => (
            false,
            format!("not read: {e}"),
            "its database does not read".to_string(),
        ),
        Ok(None) => {
            let why = format!(
                "no tick in {} min: the job is not running",
                STALE_MS / 60_000
            );
            (false, why.clone(), why)
        }
        Ok(Some((s, at))) => {
            let ago = format!("{s} ({} s ago)", (now_ms - at) / 1000);
            match s.strip_prefix("down: ") {
                Some(why) => (false, ago, why.to_string()),
                None => (true, ago, "none".into()),
            }
        }
    };
    vec![
        format!("State: {}", if ok { "ok" } else { "NOT OK" }),
        format!("Runner: one Hostman Cloud Server, label {LABEL}, kept up each minute"),
        format!("Status: {status}"),
        format!("Alarm: {alarm}"),
    ]
}

const TITLE: &str = "CI runner";

fn tree(lines: &[String]) -> String {
    let mut elements = serde_json::Map::new();
    for (i, line) in lines.iter().enumerate() {
        elements.insert(
            format!("l{i}"),
            json!({"type": "text", "props": {"text": line}}),
        );
    }
    let names: Vec<String> = (0..lines.len()).map(|i| format!("l{i}")).collect();
    elements.insert(
        "page".into(),
        json!({"type": "section", "props": {"title": TITLE}, "children": names}),
    );
    json!({"root": "page", "elements": elements}).to_string()
}

// One URL, both forms (rust/CLAUDE.md point 4), beside /status/ci-queue and /status/memory: the older app's /status keeps its
// own rows until that page is ported whole.
inventory::submit! {
    Page { path: "/status/ci-runner", step: "route_status_ci_runner", reads: Reads::Nothing, redraw: true, serve: |a| {
        let now = a.host.now_ms();
        let read = a.host.open(&a.data(), FILE, &[SCHEMA]).and_then(|log| status(&*log, now));
        a.form(Some(Ok(Drawing::Tree { title: TITLE.into(), tree: tree(&lines(read, now)) })))
    } }
}
