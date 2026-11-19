//! The runner kept up live (task rust-hostman-ci-wire; @nha, 2026-11-14: « If CI is the bottleneck... use hostman as I
//! asked. », « take one 8GB machine please. »): `Runner::tick` registered as a recurring job the host's queue runs each
//! minute (dash_jobs), its rows in this plugin's own database, and /status/ci-runner showing them, its alarm when Down. The
//! hostman tool is called through the host's registry, so its Cloud-Servers-only scope, its credit cap and its audit log hold;
//! GitHub is asked through the host's `forge` (his gh login), so no secret lives here.
use crate::evidence::Budget;
use crate::{LIGHT, MODEL, NO_RUNNER, REINSTALL_MS, Reg, Runner, Seen, State, VANISHED, Want};
use dash_core::{Drawing, Host, OpLog, Page, Reads, Recurring, Tick, Value, json};
use serde_json::Value as Json;

/// This plugin's database, under the host's data folder.
pub const FILE: &str = "hostman-ci.sqlite";
/// `ci_box` beside it, not a column added: a table made before holds no `ip`, and CREATE TABLE IF NOT EXISTS adds none.
pub const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS ci_runner (name TEXT PRIMARY KEY, status TEXT, alarm TEXT) WITHOUT ROWID; \
                          CREATE TABLE IF NOT EXISTS ci_box (name TEXT PRIMARY KEY, ip TEXT) WITHOUT ROWID; \
                          CREATE TABLE IF NOT EXISTS ci_credit (name TEXT PRIMARY KEY, balance TEXT) WITHOUT ROWID;";
/// The finances each tick reads through the hostman tool, read only: its balance is the credit left, in euros by his word
/// (@nha, 2026-11-18: « euros. the API is wrong. »), which `dash infra plan`'s guard reads from these rows (rfc-iac S4).
pub const FINANCES: &str = r#"{"method":"GET","path":"/api/v1/account/finances"}"#;

/// The balance's row from the finances answer; none when it holds no balance (an error is no reading, never a zero).
pub fn credit_row(answer: &str) -> Option<dash_core::FieldWrite> {
    let balance = serde_json::from_str::<Json>(answer).ok()?["finances"]["balance"].as_f64()?;
    Some(dash_core::FieldWrite {
        table: "ci_credit".into(),
        row: "hostman".into(),
        field: "balance".into(),
        value: Value::Text(balance.to_string()),
    })
}
/// What the box's runners register to: this repository, and Klar's organisation, whose klar-im/internal is outside nha-org
/// (task hostman-ci-klar-runner; @nha, 2026-11-15: « it runs Klar's CI on the same »).
pub const REGISTRATIONS: [&str; 2] = ["nha-org/site_perso", "klar-im"];

/// The boxes kept up (task ci-runner-pool; @nha, 2026-11-18: « we can probably spin up a second server. », « I suggest
/// spinning the second server and seeing how it holds. »): ci, 8 GB, its runners under `LABEL` for both registrations and
/// one more for this repository under `LIGHT`; ci-2, 4 GB, one runner for this repository under `LABEL`.
pub fn pool() -> Vec<Want> {
    let mut big = Want::eight_gb(&REGISTRATIONS);
    big.registrations.push(Reg {
        to: REGISTRATIONS[0].into(),
        label: LIGHT,
    });
    let pool = vec![big, Want::four_gb(&REGISTRATIONS[..1])];
    // the light runner's name is its label (`Reg::runner`): a second box with one would replace the first on GitHub
    let light = |w: &&Want| w.registrations.iter().any(|r| r.label == LIGHT);
    assert_eq!(
        pool.iter().filter(light).count(),
        1,
        "one box carries the light runner"
    );
    pool
}

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

/// This tick's runner lists, by registration: the boxes' runners of one registration read it once a tick, never once each
/// (/code-review: three identical reads a minute for site_perso). Emptied as each tick starts, so no list outlives its
/// tick; an error is never kept.
#[derive(Default)]
pub struct Lists(std::cell::RefCell<std::collections::HashMap<String, String>>);

impl Lists {
    pub fn get(
        &self,
        key: &str,
        read: impl FnOnce() -> Result<String, String>,
    ) -> Result<String, String> {
        if let Some(list) = self.0.borrow().get(key) {
            return Ok(list.clone());
        }
        let list = read()?;
        self.0.borrow_mut().insert(key.into(), list.clone());
        Ok(list)
    }

    /// A new tick: every list is read again.
    pub fn clear(&self) {
        self.0.borrow_mut().clear();
    }
}

/// One `Runner` a box of the pool, each tick of the job ticking each in turn, its rows under its own name.
fn start(host: &'static dyn Host) -> Result<Tick, String> {
    let log = host.log(&host.data(), FILE, &[SCHEMA])?;
    let lists = std::rc::Rc::new(Lists::default());
    // one 90 s for the whole tick's captures, however many boxes it reinstalls
    let began = std::time::Instant::now();
    let budget = std::rc::Rc::new(Budget::new(Box::new(move || began.elapsed().as_secs())));
    let mut runners = vec![];
    for want in pool() {
        let (name, lists) = (want.name.clone(), std::rc::Rc::clone(&lists));
        let (box_name, budget_) = (name.clone(), std::rc::Rc::clone(&budget));
        let state = seed(&*log, host.now_ms(), &want)?;
        let runner = Runner::new(
            want,
            Box::new(move |asked| host.call("hostman", asked)),
            Box::new(move |r: &Reg| {
                let runners = format!("{}/actions/runners?per_page=100", api(&r.to));
                let read = || host.forge(&["api", &runners]);
                let listed = lists.get(&runners, read)?;
                online(&listed, &r.runner(&name), r.label)
            }),
            Box::new(move |r: &Reg| {
                let token = format!("{}/actions/runners/registration-token", api(&r.to));
                let t = host.forge(&["api", "-X", "POST", &token, "--jq", ".token"])?;
                Some(t.trim().to_string())
                    .filter(|t| !t.is_empty())
                    .ok_or("GitHub gave no registration token".into())
            }),
        )
        .clocked(Box::new(move || host.now_ms()))
        .keyed(Box::new(move || box_keys(host)))
        .evidenced(Box::new(move |id, ip| {
            evidence(host, &budget_, &box_name, id, ip)
        }))
        .seeded(state);
        runners.push(runner);
    }
    Ok(Box::new(move || {
        lists.clear();
        budget.start();
        let mut rows: Vec<_> = runners.iter().flat_map(Runner::tick).collect();
        rows.extend(
            host.call("hostman", FINANCES)
                .ok()
                .and_then(|a| credit_row(&a)),
        );
        rows
    }))
}

/// The reconciler's own ssh key, made once under the data folder (`evidence::own_key`): its private key's path, the public
/// key beside it. A background service never uses his personal key.
fn own_key(host: &dyn Host) -> Result<(std::path::PathBuf, String), String> {
    let folder = host.data().join("hostman-ci-ssh");
    let keygen_ = |args: &[&str]| -> Result<String, String> {
        let limit = std::time::Duration::from_secs(20);
        let program = dash_core::Program {
            bin: "ssh-keygen",
            args,
            dir: None,
            given: &[],
            stdin: None,
            limit,
        };
        let ran = host.run(&program)?;
        match ran.code {
            0 => Ok(ran.stdout),
            code => Err(format!("ssh-keygen exited {code}: {}", ran.stderr.trim())),
        }
    };
    let keygen = |key: &std::path::Path| -> Result<(), String> {
        let path = key.display().to_string();
        keygen_(&[
            "-q",
            "-t",
            "ed25519",
            "-N",
            "",
            "-C",
            "hostman-ci",
            "-f",
            &path,
        ])
        .map(drop)
    };
    let derive = |key: &std::path::Path| keygen_(&["-y", "-f", &key.display().to_string()]);
    let public = crate::evidence::own_key(&folder, &keygen, &derive)?;
    Ok((folder.join("id_ed25519"), public))
}

/// His ssh public keys (`keys`) and the reconciler's own, which root takes beside his so a heal's capture reaches the box.
/// The reconciler's key that cannot be made is a named error, the keys unread (a Down, its own status), never dropped from
/// the cloud-init in silence.
fn box_keys(host: &dyn Host) -> Result<Vec<String>, String> {
    let mut all = keys(&host.home()?)?;
    all.push(own_key(host)?.1);
    Ok(all)
}

/// A reinstall's capture of box `name` (server `id`, on, at `ip`) before it wipes it (`evidence::gather`, task
/// ci-heal-evidence), within what is left of the tick's `budget`: Hostman's view of it through the hostman tool (its state,
/// its statistics of the last two hours, its log), then the box itself over ssh, with the reconciler's own known-hosts file
/// in the data folder, never his ~/.ssh/known_hosts. Kept in the repository's private/live/ci-heal/, outside git: « evidence
/// in » the file, or why none.
pub fn evidence(host: &dyn Host, budget: &Budget, name: &str, id: i64, ip: &str) -> String {
    use crate::evidence::{Read, SPENT, STEP_S, kept, ssh_args, ssh_said};
    let now_s = host.now_ms() / 1000;
    let at = |s: i64| dash_core::iso(s).trim_end_matches('Z').to_string();
    let tool = |path: String| {
        host.call(
            "hostman",
            &json!({"method": "GET", "path": path}).to_string(),
        )
    };
    // its own known-hosts file, emptied first: each reinstall gives the box a new host key, which a kept entry would refuse
    let known = host.data().join("hostman-ci-known-hosts");
    let ssh = |limit_s: u64| -> Result<String, String> {
        let (key, _) = own_key(host)?;
        std::fs::write(&known, "").map_err(|e| format!("{}: {e}", known.display()))?;
        let args = ssh_args(&key, &known, ip);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let limit = std::time::Duration::from_secs(limit_s);
        let program = dash_core::Program {
            bin: "ssh",
            args: &args,
            dir: None,
            given: &[],
            stdin: None,
            limit,
        };
        let ran = host.run(&program)?;
        ssh_said(ran.code, &ran.stderr, &ran.stdout)
    };
    // the hostman tool's calls hold their own bound, curl's --max-time 20, whatever they are given; ssh takes its limit
    let steps: Vec<Read> = vec![
        (
            "hostman: the server",
            STEP_S,
            Box::new(|_| tool(format!("/api/v1/servers/{id}"))),
        ),
        (
            "hostman: its statistics, the last two hours",
            STEP_S,
            Box::new(|_| {
                tool(format!(
                    "/api/v1/servers/{id}/statistics?date_from={}&date_to={}",
                    at(now_s - 7200),
                    at(now_s)
                ))
            }),
        ),
        (
            "hostman: its log",
            STEP_S,
            Box::new(|_| tool(format!("/api/v1/servers/{id}/logs?limit=30"))),
        ),
        ("the box over ssh", 0, Box::new(ssh)),
    ];
    let Some(text) = budget.capture(steps) else {
        return SPENT.into();
    };
    let utc = dash_core::iso(now_s);
    let dir = host.repo().join("private/live/ci-heal");
    let head = format!("{name}, server {id}, captured before its reinstall at {utc}\n\n{text}");
    match kept(&dir, name, &utc, &head) {
        Ok(file) => format!("evidence in {file}"),
        Err(e) => format!("evidence not written ({e})"),
    }
}

/// The state a restart resumes, from the statuses of the last `REINSTALL_MS`: when the last reinstall was, and since when
/// the server has been on with `NO_RUNNER`, the start of the last run of such statuses. The app redeploys every few
/// minutes some days (09:27 and 09:38 on 2026-11-15): a state held in memory alone would never reach 15 minutes. No runner
/// is resumed as seen (`State::seen`): a death across a restart waits the hour, never a false heal.
/// ponytail: the times are read back from the statuses' words; a column each, if a wording change ever breaks one.
pub fn seed(log: &dyn OpLog, now_ms: i64, want: &Want) -> Result<State, String> {
    let mut state = State::default();
    each_status(log, now_ms - REINSTALL_MS, &want.name, &mut |text, at| {
        let reinstalled = text.starts_with("reinstalled");
        if reinstalled {
            state.installed = Some(at);
            // a heal is resumed as the reinstall is: a restart never lets a second heal in within the hour
            if text.contains(VANISHED) {
                state.healed = Some(at);
            }
        }
        // a reinstall's reason may name NO_RUNNER: the wait starts after it, as the live reconcile's does
        state.since = match text.contains(NO_RUNNER) && !reinstalled {
            true => state.since.or(Some(at)),
            false => None,
        };
    })?;
    Ok(state)
}

dash_core::submit! {
    Recurring { model: MODEL, every_ms: EVERY_MS, file: FILE, schema: &[SCHEMA], start }
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

/// The runner `name` with `label` in GitHub's runner list: online, busy or not. The name too: the whatsnr VPS
/// runners carried the old label as a bridge, so a label alone read « up » a minute after server 9602127 was made, and its
/// missing runner was never healed (2026-11-15, 01:56 to 08:42).
pub fn online(listed: &str, name: &str, label: &str) -> Result<Seen, String> {
    let j: Json = serde_json::from_str(listed).map_err(|_| "GitHub's runner list is no JSON")?;
    let runners = j["runners"]
        .as_array()
        .ok_or("GitHub's runner list holds no runners")?;
    let labelled =
        |r: &Json| (r["labels"].as_array()).is_some_and(|l| l.iter().any(|l| l["name"] == label));
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

/// Each status the job wrote for box `name` since `from_ms`, oldest first, with when, read a page at a time.
fn each_status(
    log: &dyn OpLog,
    from_ms: i64,
    name: &str,
    f: &mut dyn FnMut(&str, i64),
) -> Result<(), String> {
    each(log, ("ci_runner", "status", name), from_ms, f)
}

/// Each text the job wrote in `table`'s `field` of row `name` since `from_ms`, oldest first, with when, read a page at a time.
fn each(
    log: &dyn OpLog,
    (table, field, name): (&str, &str, &str),
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
                op.write.table == table && op.write.field == field && op.write.row == name,
                &op.write.value,
            ) {
                f(t, op.stamp >> 16);
            }
        }
    }
}

/// The last status the job wrote for box `name` within `STALE_MS` of `now_ms`, read from the log's recent ops alone (a page
/// at a time is the rule: here the window is the page); none when the job wrote nothing in that time.
pub fn status(log: &dyn OpLog, now_ms: i64, name: &str) -> Result<Option<(String, i64)>, String> {
    let mut last = None;
    each_status(log, now_ms - STALE_MS, name, &mut |t, at| {
        last = Some((t.to_string(), at))
    })?;
    Ok(last)
}

/// The page's line for box `name`'s address, as the job last listed it within `STALE_MS`: what `ssh root@` takes.
pub fn address(log: &dyn OpLog, now_ms: i64, name: &str) -> Result<String, String> {
    let mut last = None;
    each(
        log,
        ("ci_box", "ip", name),
        now_ms - STALE_MS,
        &mut |t, _| last = Some(t.to_string()),
    )?;
    Ok(match last {
        Some(ip) => format!("Address: ssh root@{ip}"),
        None => "Address: none listed in the last ticks".into(),
    })
}

/// Whether a box of the pool is up as the job last saw it, within `STALE_MS`: the router's room for the label.
pub fn up(host: &dyn Host) -> Result<bool, String> {
    let log = host.log(&host.data(), FILE, &[SCHEMA])?;
    let mut any = false;
    for want in pool() {
        // « up: » is a box held for a reinstall, its runners taking jobs meanwhile
        let s = status(&*log, host.now_ms(), &want.name)?;
        any |= s.is_some_and(|(s, _)| s == "up" || s.starts_with("up: "));
    }
    Ok(any)
}

/// The page's lines for box `want`: its state, its runners, the status and its age, the alarm.
pub fn lines(want: &Want, read: Result<Option<(String, i64)>, String>, now_ms: i64) -> Vec<String> {
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
            "Box: Hostman Cloud Server {}, {} GB, a runner each for {}, kept up each minute",
            want.name,
            want.ram_mb / 1024,
            (want
                .registrations
                .iter()
                .map(|r| format!("{} under {}", r.to, r.label)))
            .collect::<Vec<_>>()
            .join(", ")
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
dash_core::submit! {
    Page { path: "/status/ci-runner", step: "route_status_ci_runner", reads: Reads::Nothing, redraw: true, serve: |a| {
        let now = a.host.now_ms();
        let log = a.host.log(&a.data(), FILE, &[SCHEMA]);
        // each box's lines in turn, its address last
        let mut all = vec![];
        for want in pool() {
            let read = log.as_ref().map_err(Clone::clone).and_then(|log| status(&**log, now, &want.name));
            all.extend(lines(&want, read, now));
            all.extend(log.as_ref().ok().and_then(|log| address(&**log, now, &want.name).ok()));
        }
        a.form(Some(Ok(Drawing::Tree { title: TITLE.into(), tree: tree(&all) })))
    } }
}
