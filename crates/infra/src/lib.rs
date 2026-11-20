//! dash-infra, a plugin: OpenTofu wrapped as `dash infra plan|state|graph` (rfc-iac; @nha, 2026-11-18: « Decided: OpenTofu
//! is the engine, wrapped by our Rust layer. -> agreed. »). Its tool reads tofu's JSON and answers the contract
//! (tool.rs); this root finds the unit and its guard, reads the balance, and runs tofu through the host's `run`, in its
//! own process group with only the environment it is given; its UI comes with RFC-0024's core/infra, which reads the
//! same JSON (rust/ARCHITECTURE.md section 2 point 5).
use dash_sdk::{Host, OpLog, Program, Value as Text};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tool::{Balance, ERROR, GUARD, Guard};

pub mod tool;
pub mod ui;

pub const USAGE: &str = "dash infra plan|state|graph <unit under infra/>  OpenTofu under infra/guard.json, one JSON \
                         document on stdout: exit 0 no change, 2 changes, 3 refused by the guard, 1 an error";
/// hostman-ci's database and the one table this plugin reads of it: the balance its job reads from Hostman each tick.
pub const HOSTMAN_CI: &str = "hostman-ci.sqlite";
pub const CREDIT_SCHEMA: &str =
    "CREATE TABLE IF NOT EXISTS ci_credit (name TEXT PRIMARY KEY, balance TEXT) WITHOUT ROWID;";
/// Where a unit's local backend keeps its state, outside the tree (each unit's main.tf names its file there).
pub const STATES: &str = "private/live/infra";

dash_sdk::submit! {
    dash_sdk::Command {
        usage: USAGE,
        words: &["infra"],
        run: Some(|host, args, out| match run(host, args, out) {
            0 => {}
            code => std::process::exit(code),
        }),
    }
}

/// The last balance hostman-ci's job recorded in `log` after `since_ms`, and when.
pub fn balance(log: &dyn OpLog, since_ms: i64) -> Result<Option<Balance>, String> {
    let (mut after, mut last) = ((since_ms << 16, String::new()), None);
    loop {
        let ops = log.ops_after(Some((after.0, &after.1)), 500)?;
        let Some(op) = ops.last() else {
            return Ok(last);
        };
        after = (op.stamp, op.writer.clone());
        for op in &ops {
            let w = &op.write;
            if let (true, Text::Text(t)) = (
                w.table == "ci_credit" && w.field == "balance" && w.row == "hostman",
                &w.value,
            ) && let Ok(amount) = t.parse()
            {
                last = Some(Balance {
                    amount,
                    at_ms: op.stamp >> 16,
                });
            }
        }
    }
}

/// The unit's folder and its name under the checkout's infra/: anything else is refused, and so is a unit carrying a
/// guard of its own (the central one alone counts, reviewed with its PR).
fn unit(host: &dyn Host, dir: &str) -> Result<(PathBuf, String), String> {
    let infra = host.repo().join("infra");
    let infra = infra
        .canonicalize()
        .map_err(|e| format!("{}: {e}", infra.display()))?;
    // a relative unit is the checkout's (infra/hostman/ci), wherever dash runs: never the current folder's
    let dir = host
        .repo()
        .join(dir)
        .canonicalize()
        .map_err(|e| format!("{dir}: {e}"))?;
    let name = dir
        .strip_prefix(&infra)
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .filter(|n| !n.is_empty())
        .ok_or(format!(
            "{}: a unit is a folder under {}",
            dir.display(),
            infra.display()
        ))?;
    if dir.join(GUARD).exists() {
        return Err(format!(
            "{name}: a unit's own {GUARD} is refused; infra/{GUARD} alone guards it"
        ));
    }
    Ok((dir, name))
}

/// The environment tofu gets beyond PATH, HOME and TMPDIR: automation mode, and the unit's provider secrets, each read with
/// bin/secret for this child alone (`secrets` in infra/guard.json, by the unit's provider folder).
fn environment(
    host: &dyn Host,
    rules: &Guard,
    unit: &str,
) -> Result<Vec<(String, String)>, String> {
    let mut env = vec![
        ("TF_IN_AUTOMATION".into(), "1".into()),
        ("TF_INPUT".into(), "0".into()),
    ];
    let provider = unit.split('/').next().unwrap_or_default();
    let secret = host.repo().join("bin/secret").display().to_string();
    for (var, words) in rules
        .secrets
        .get(provider)
        .and_then(|s| s.as_object())
        .into_iter()
        .flatten()
    {
        let words: Vec<&str> = words
            .as_str()
            .unwrap_or_default()
            .split_whitespace()
            .collect();
        let args: Vec<&str> = ["get"].into_iter().chain(words).collect();
        let ran = host.run(&Program {
            bin: &secret,
            args: &args,
            dir: None,
            given: &[],
            stdin: None,
            limit: Duration::from_secs(30),
        })?;
        match (ran.code, ran.stdout.trim()) {
            (0, value) if !value.is_empty() => env.push((var.clone(), value.to_string())),
            _ => return Err(format!("{unit}: bin/secret gave no {var}")),
        }
    }
    Ok(env)
}

/// The states' folder outside the tree, its own alone (0700), each state there 0600: a state holds every value in clear.
fn private_states(host: &dyn Host) {
    use std::os::unix::fs::PermissionsExt;
    let dir = host.repo().join(STATES);
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let _ = std::fs::set_permissions(entry.path(), std::fs::Permissions::from_mode(0o600));
        // a saved plan an hour old is a cut run's: its tofu, in a group of its own, may have written it after dash ended
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|t| t.elapsed().is_ok_and(|e| e > Duration::from_secs(3600)));
        if stale && entry.file_name().to_string_lossy().ends_with(".tfplan") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// On SIGINT, SIGTERM or SIGHUP, the saved plan is removed before this process ends: a Drop never runs on a signal. The
/// plan sits in the states' folder (0700, outside the tree), so a tofu that outlives dash and writes it later leaves it
/// there, removed by the next run (`private_states`), never in the checkout.
fn removed_on_signal(saved: PathBuf) {
    use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
    let Ok(mut signals) = signal_hook::iterator::Signals::new([SIGINT, SIGTERM, SIGHUP]) else {
        return;
    };
    std::thread::spawn(move || {
        if let Some(signal) = signals.forever().next() {
            let _ = std::fs::remove_file(&saved);
            std::process::exit(128 + signal);
        }
    });
}

/// `dash infra`: the words after it, the document on `out`, the exit code returned.
pub fn run(host: &dyn Host, args: &[&str], out: &mut dyn Write) -> i32 {
    match answer(host, args) {
        Ok((doc, code)) => match writeln!(
            out,
            "{}",
            serde_json::to_string_pretty(&doc).unwrap_or_default()
        ) {
            Ok(()) => code,
            Err(e) => {
                eprintln!("dash infra: stdout: {e}");
                ERROR
            }
        },
        Err(why) => {
            eprintln!("dash infra: {why}");
            ERROR
        }
    }
}

fn answer(host: &dyn Host, args: &[&str]) -> Result<(serde_json::Value, i32), String> {
    let [word @ ("plan" | "state" | "graph"), dir] = args else {
        return Err(format!("usage: {USAGE}"));
    };
    let (dir, name) = unit(host, dir)?;
    let guard = host.repo().join("infra").join(GUARD);
    let text = std::fs::read_to_string(&guard).map_err(|e| format!("{}: {e}", guard.display()))?;
    let rules = Guard::parse(&text)?;
    let env = environment(host, &rules, &name)?;
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let tofu = |dir: &Path, args: &[&str], limit: u64| {
        let program = Program {
            bin: "tofu",
            args,
            dir: Some(dir),
            given: &env,
            stdin: None,
            limit: Duration::from_secs(limit),
        };
        host.run(&program).map(|r| (r.code, r.stdout, r.stderr))
    };
    private_states(host);
    let now = host.now_ms();
    let answered = match *word {
        "plan" => {
            let since = now - tool::BALANCE_MS - 3_600_000;
            let read = host.log(&host.data(), HOSTMAN_CI, &[CREDIT_SCHEMA]).and_then(|log| balance(&*log, since));
            let file = format!("{}.{}.tfplan", name.replace('/', "-"), std::process::id());
            let saved = host.repo().join(STATES).join(file);
            removed_on_signal(saved.clone());
            tool::plan(&tofu, &dir, &name, &rules, read.ok().flatten(), now, &saved)
        }
        "state" => tool::shown_state(&tofu, &dir)
            .map(|s| (serde_json::json!({ "contract": tool::CONTRACT, "unit": name, "resources": tool::resources(&s) }), 0)),
        _ => tool::shown_state(&tofu, &dir).map(|s| {
            let mut g = tool::graph(&s);
            g["contract"] = serde_json::json!(tool::CONTRACT);
            g["unit"] = serde_json::json!(name);
            (g, 0)
        }),
    };
    private_states(host);
    answered
}

#[cfg(test)]
mod tests;
