//! The runner kept up live (task rust-hostman-ci-wire; @nha, 2026-11-14: « If CI is the bottleneck... use hostman as I
//! asked. », « take one 8GB machine please. »): `Runner::tick` registered as a recurring job the host's queue runs each
//! minute (dash_jobs), its rows in this plugin's own database, and /status/ci-runner showing them, its alarm when Down. The
//! hostman tool is called through the host's registry, so its Cloud-Servers-only scope, its credit cap and its audit log hold;
//! GitHub is asked through the host's `forge` (his gh login), so no secret lives here.
use crate::{LABEL, MODEL, NO_RUNNER, REINSTALL_MS, Runner, Seen, State, Want};
use dash_core::{Drawing, Host, OpLog, Page, Reads, Recurring, Tick, Value, json};
use serde_json::Value as Json;

/// This plugin's database, under the host's data folder.
pub const FILE: &str = "hostman-ci.sqlite";
/// `ci_box` beside it, not a column added: a table made before holds no `ip`, and CREATE TABLE IF NOT EXISTS adds none.
pub const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS ci_runner (name TEXT PRIMARY KEY, status TEXT, alarm TEXT) WITHOUT ROWID; \
                          CREATE TABLE IF NOT EXISTS ci_box (name TEXT PRIMARY KEY, ip TEXT) WITHOUT ROWID;";
/// What the box's runners register to: this repository, and Klar's organisation, whose klar-im/internal is outside nha-org
/// (task hostman-ci-klar-runner; @nha, 2026-11-15: « it runs Klar's CI on the same »).
pub const REGISTRATIONS: [&str; 2] = ["nha-org/site_perso", "klar-im"];

/// GitHub's API path of a registration: `repos/owner/name` for a repository, `orgs/name` for an organisation.
pub fn api(registration: &str) -> String {
    match registration.contains('/') {
        true => format!("repos/{registration}"),
        false => format!("orgs/{registration}"),
    }
}
const EVERY_MS: i64 = 60_000;
/// A status older than this is no reading: the job is not running, and the page says so (rust/CLAUDE.md point 5).
pub const STALE_MS: i64 = 5 * EVERY_MS;

fn start(host: &'static dyn Host) -> Result<Tick, String> {
    let want = Want::eight_gb(&REGISTRATIONS);
    let name = want.name.clone();
    let log = host.log(&host.data(), FILE, &[SCHEMA])?;
    let state = seed(&*log, host.now_ms())?;
    let runner = Runner::new(
        want,
        Box::new(move |asked| host.call("hostman", asked)),
        Box::new(move |r| {
            let runners = format!("{}/actions/runners?per_page=100", api(r));
            online(&host.forge(&["api", &runners])?, &name)
        }),
        Box::new(move |r| {
            let token = format!("{}/actions/runners/registration-token", api(r));
            let t = host.forge(&["api", "-X", "POST", &token, "--jq", ".token"])?;
            Some(t.trim().to_string())
                .filter(|t| !t.is_empty())
                .ok_or("GitHub gave no registration token".into())
        }),
    )
    .clocked(Box::new(move || host.now_ms()))
    .keyed(Box::new(move || keys(&host.home()?)))
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

/// His ssh public keys, `~/.ssh/id_*.pub`, read at each tick so a new one reaches the box (task ci-box-open-and-current,
/// done-when 1: never written in rust/). His own keys alone: a deploy key beside them, as graal-ci-deploy.pub, is held by
/// another CI too, which root on this box is not for. None read is an error, never a box remade with no key.
pub fn keys(home: &std::path::Path) -> Result<Vec<String>, String> {
    let dir = home.join(".ssh");
    let listed = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut files: Vec<_> = (listed.filter_map(Result::ok).map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("id_") && n.ends_with(".pub"))
        })
        .collect();
    files.sort();
    let mut keys = vec![];
    for f in files {
        let text = std::fs::read_to_string(&f).map_err(|e| format!("{}: {e}", f.display()))?;
        keys.extend(
            (text.lines().map(str::trim))
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(String::from),
        );
    }
    match keys.is_empty() {
        true => Err(format!("no public key in {}/id_*.pub", dir.display())),
        false => Ok(keys),
    }
}

/// The runner `name` with `LABEL` in GitHub's runner list: online, busy or not. The name too: the whatsnr VPS
/// runners carried the old label as a bridge, so a label alone read « up » a minute after server 9602127 was made, and its
/// missing runner was never healed (2026-11-15, 01:56 to 08:42).
pub fn online(listed: &str, name: &str) -> Result<Seen, String> {
    let j: Json = serde_json::from_str(listed).map_err(|_| "GitHub's runner list is no JSON")?;
    let runners = j["runners"]
        .as_array()
        .ok_or("GitHub's runner list holds no runners")?;
    let labelled =
        |r: &Json| (r["labels"].as_array()).is_some_and(|l| l.iter().any(|l| l["name"] == LABEL));
    let ours: Vec<&Json> = (runners.iter())
        .filter(|r| r["status"] == "online" && r["name"] == name && labelled(r))
        .collect();
    Ok(
        match (ours.is_empty(), ours.iter().any(|r| r["busy"] == true)) {
            (true, _) => Seen::Gone,
            (false, true) => Seen::Busy,
            (false, false) => Seen::Idle,
        },
    )
}

/// Each status the job wrote since `from_ms`, oldest first, with when, read a page at a time.
fn each_status(log: &dyn OpLog, from_ms: i64, f: &mut dyn FnMut(&str, i64)) -> Result<(), String> {
    each(log, ("ci_runner", "status"), from_ms, f)
}

/// Each text the job wrote in `table`'s `field` since `from_ms`, oldest first, with when, read a page at a time.
fn each(
    log: &dyn OpLog,
    (table, field): (&str, &str),
    from_ms: i64,
    f: &mut dyn FnMut(&str, i64),
) -> Result<(), String> {
    let mut after = (from_ms << 16, String::new());
    loop {
        let ops = log.ops_after(Some((after.0, &after.1)), 500)?;
        let Some(op) = ops.last() else {
            return Ok(());
        };
        after = (op.stamp, op.writer.clone());
        for op in &ops {
            if let (true, Value::Text(t)) = (
                op.write.table == table && op.write.field == field,
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

/// The page's line for the box's address, as the job last listed it within `STALE_MS`: what `ssh root@` takes.
pub fn address(log: &dyn OpLog, now_ms: i64) -> Result<String, String> {
    let mut last = None;
    each(log, ("ci_box", "ip"), now_ms - STALE_MS, &mut |t, _| {
        last = Some(t.to_string())
    })?;
    Ok(match last {
        Some(ip) => format!("Address: ssh root@{ip}"),
        None => "Address: none listed in the last ticks".into(),
    })
}

/// Whether the runner is up as the job last saw it, within `STALE_MS`: the router's room for it.
pub fn up(host: &dyn Host) -> Result<bool, String> {
    let log = host.log(&host.data(), FILE, &[SCHEMA])?;
    // « up: » is a box held for a reinstall, its runners taking jobs meanwhile
    Ok(status(&*log, host.now_ms())?.is_some_and(|(s, _)| s == "up" || s.starts_with("up: ")))
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
        format!(
            "Runner: one Hostman Cloud Server, label {LABEL}, a runner each for {}, kept up each minute",
            REGISTRATIONS.join(" and ")
        ),
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
        let log = a.host.log(&a.data(), FILE, &[SCHEMA]);
        let read = log.as_ref().map_err(Clone::clone).and_then(|log| status(&**log, now));
        let mut lines = lines(read, now);
        lines.extend(log.ok().and_then(|log| address(&*log, now).ok()));
        a.form(Some(Ok(Drawing::Tree { title: TITLE.into(), tree: tree(&lines) })))
    } }
}
