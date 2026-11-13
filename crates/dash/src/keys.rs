//! `dash agents`: section 20b's key rows (rust-key-agent). A row names a bin/secret name, never a value. A run on it is headless
//! (the orchestrator's answer in its question dialog, 2026-11-08: « A: headless run »): the value in that run's environment
//! alone, never on a command line, in the store or in what it prints. Its spend is our count, held to the row's cap.
use dash_agent::{headless, headless_outcome, key_var};
use dash_core::tasks::Limit;
use dash_store::keys::{self, Key, Keys};
use serde_json::Value;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// bin/secret answers from a file: a minute is a hang.
const SECRET_LIMIT: Duration = Duration::from_secs(60);
/// A headless run is one turn, its tools included.
const RUN_LIMIT: Duration = Duration::from_secs(30 * 60);

/// The programs a key's commands call: bin/secret and the kind's tool. Tests name stand-ins.
pub struct Tools {
    pub secret: String,
    pub claude: String,
}

/// `repo`: the checkout whose bin/secret holds the values. `DASH_CLAUDE` names another tool.
pub fn live_tools(repo: &Path) -> Tools {
    let secret = repo.join("bin/secret").display().to_string();
    let claude = std::env::var("DASH_CLAUDE").unwrap_or("claude".into());
    Tools { secret, claude }
}

fn or_exit<T>(result: Result<T, String>) -> T {
    result.unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    })
}

fn line(key: &Key) -> String {
    let Key {
        id,
        secret,
        cap,
        spent,
    } = key;
    format!("{id}  secret {secret}  spent ${spent:.4} of a ${cap:.2} cap")
}

/// The words of a refused name, never repeating it: it may be the key itself.
fn not_a_name(kind: &str, var: &str) -> String {
    format!(
        "{kind}: refused, the secret must be a name bin/secret list shows holding {var}, never a key's value; what was given is not repeated"
    )
}

/// Whether bin/secret holds `name` with `var` among its keys.
fn holds(tools: &Tools, name: &str, var: &str) -> Result<bool, String> {
    let listed = dash_process::run(&tools.secret, &["list", "--json"], None, SECRET_LIMIT)?;
    let listed: Value =
        serde_json::from_str(&listed).map_err(|e| format!("bin/secret list: {e}"))?;
    let entries = listed.as_array().map(Vec::as_slice).unwrap_or_default();
    Ok(entries.iter().any(|e| {
        let keys = e["keys"].as_array().map(Vec::as_slice).unwrap_or_default();
        e["name"] == name && keys.iter().any(|k| k == var)
    }))
}

fn kind_var(kind: &str) -> Result<&'static str, String> {
    key_var(kind).ok_or(format!("{kind}: no such key kind; the kinds: anthropic"))
}

/// `cap`: none keeps the row's own, 0 for a new row.
pub fn added(
    keys: &mut Keys,
    tools: &Tools,
    kind: &str,
    name: &str,
    cap: Option<f64>,
) -> Result<String, String> {
    let var = kind_var(kind)?;
    let had = keys.list()?.into_iter().find(|k| k.id == kind);
    let cap = cap.unwrap_or(had.map_or(0.0, |k| k.cap));
    if !(cap.is_finite() && cap >= 0.0) {
        return Err(format!("{kind}: refused, a cap is US dollars, 0 or more"));
    }
    if !holds(tools, name, var)? {
        return Err(not_a_name(kind, var));
    }
    keys.add(kind, name, cap)?;
    let key = keys.list()?.into_iter().find(|k| k.id == kind);
    key.map(|k| line(&k))
        .ok_or(format!("{kind}: written, then not read back"))
}

/// Every `value` in `text` blanked: a tool may echo its environment, and what it says goes to the store and the screen.
fn blank(text: &str, value: &str) -> String {
    text.replace(value, "[key]")
}

/// One headless run on `kind`'s row, its spend counted, under the row's lock: a second run at once is refused, so no two runs
/// pass the cap's check on one reading. Refused once the row's spend reached its cap; the run itself may spend only the rest.
pub fn ran(
    keys: &mut Keys,
    tools: &Tools,
    data: &Path,
    kind: &str,
    prompt: &str,
) -> Result<String, String> {
    let var = kind_var(kind)?;
    // `kind` is one key_var knows, so no path.
    let lock = data.join(format!("key-{kind}.lock"));
    let lock = File::create(&lock).map_err(|e| format!("{}: {e}", lock.display()))?;
    if lock.try_lock().is_err() {
        return Err(format!("{kind}: refused, a run on this key is running"));
    }
    let key = keys.list()?.into_iter().find(|k| k.id == kind);
    let key = key.ok_or(format!(
        "{kind}: no key row: dash agents key add {kind} <secret name>"
    ))?;
    if !key.limit().room() {
        return Err(format!(
            "{kind}: refused, spent ${:.4} of a ${:.2} cap: dash agents key add {kind} {} --cap <usd> raises it",
            key.spent, key.cap, key.secret
        ));
    }
    if prompt.is_empty() || prompt.starts_with('-') {
        return Err("refused: a prompt is words, never empty or a flag".into());
    }
    // Its stdout is the value: a failure names its status and stderr alone.
    let got = dash_process::answer(
        &tools.secret,
        &["get", &key.secret, var],
        None,
        &[],
        SECRET_LIMIT,
    )?;
    let value = match got {
        (status, value, _) if status.success() => value.trim().to_string(),
        (status, _, reason) => {
            return Err(format!(
                "bin/secret get {} {var}: {status}: {}",
                key.secret,
                reason.trim()
            ));
        }
    };
    if value.is_empty() {
        return Err(format!(
            "{kind}: bin/secret holds no {var} in {}",
            key.secret
        ));
    }
    let rest = key.cap - key.spent;
    let args = headless(prompt, rest);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let answer = dash_process::answer(&tools.claude, &args, None, &[(var, &value)], RUN_LIMIT);
    let at = SystemTime::now().duration_since(UNIX_EPOCH);
    let run = format!(
        "{kind}-{}-{}",
        at.map_or(0, |d| d.as_millis()),
        std::process::id()
    );
    // A run that started and then failed, or ended with no cost to read, may still have spent: it is never counted free. It
    // takes the rest of the cap, so nothing more runs on the row until he raises it. One that never started spent nothing.
    let (status, text, reason) = match answer {
        Ok(answered) => answered,
        Err(no) if !no.started => return Err(blank(&no.why, &value)),
        Err(no) => {
            keys.spend(&run, kind, rest)?;
            let said = format!(
                "{kind}: {}: its spend is not known, so the rest of its cap is counted",
                no.why
            );
            return Err(blank(&said, &value));
        }
    };
    let spent = headless_outcome(&text);
    keys.spend(&run, kind, spent.as_ref().map_or(rest, |(_, usd)| *usd))?;
    let said = match spent {
        Ok((said, usd)) => format!("{said}\nspent: ${usd:.4}"),
        Err(e) => format!(
            "its spend is not known ({e}), so the rest of its cap is counted: {}",
            text.trim()
        ),
    };
    let said = blank(&said, &value);
    if status.success() {
        return Ok(said);
    }
    Err(blank(
        &format!("{kind}: {status}: {} {said}", reason.trim()),
        &value,
    ))
}

pub fn list(dir: &Path, mut out: File) {
    let keys = or_exit(keys::open(dir).and_then(|k| k.list()));
    if keys.is_empty() {
        writeln!(out, "no key rows: dash agents key add <kind> <secret name>").expect("stdout");
    }
    for key in &keys {
        writeln!(out, "{}", line(key)).expect("stdout");
    }
}

pub fn add(dir: &Path, tools: &Tools, kind: &str, name: &str, cap: Option<&str>, mut out: File) {
    let parsed = cap.map(|c| {
        c.parse::<f64>()
            .map_err(|_| format!("{c}: not a cap in US dollars"))
    });
    let cap = or_exit(parsed.transpose());
    let mut keys = or_exit(keys::open(dir));
    let added = or_exit(added(&mut keys, tools, kind, name, cap));
    writeln!(out, "{added}").expect("stdout");
}

pub fn rm(dir: &Path, kind: &str, mut out: File) {
    let mut keys = or_exit(keys::open(dir));
    if !or_exit(keys.remove(kind)) {
        eprintln!("{kind}: no key row");
        std::process::exit(1);
    }
    writeln!(out, "{kind}: removed").expect("stdout");
}

pub fn run(dir: &Path, tools: &Tools, kind: &str, prompt: &str, mut out: File) {
    let mut keys = or_exit(keys::open(dir));
    let said = or_exit(ran(&mut keys, tools, dir, kind, prompt));
    writeln!(out, "{said}").expect("stdout");
}

/// Each key row's limit, for the Policy snapshot. A failed read costs those limits only: no key ever starts a pane.
pub fn key_limits(data: &Path) -> Vec<Limit> {
    match keys::open(data).and_then(|k| k.list()) {
        Ok(keys) => keys.iter().map(Key::limit).collect(),
        Err(e) => {
            eprintln!("key rows: {e}");
            vec![]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::dash_test;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    const VALUE: &str = "sk-ant-api03-stand-in-value";

    fn script(dir: &Path, name: &str, body: &str) -> String {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("script");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        path.display().to_string()
    }

    /// bin/secret's stand-in, holding the anthropic name and its value; the tool's, writing its argv and its environment's key
    /// beside it, then answering as `claude -p --output-format json` does.
    fn tools(dir: &Path, answer: &str, code: u8) -> Tools {
        let list = r#"[{"name":"anthropic","keys":["ANTHROPIC_API_KEY"]},{"name":"malt","keys":["MALT_ACCESS_TOKEN"]}]"#;
        let secret = script(
            dir,
            "secret",
            &format!(
                "case \"$1 $2 $3\" in\n  'list --json ') echo '{list}' ;;\n  'get anthropic ANTHROPIC_API_KEY') echo '{VALUE}' ;;\n  *) exit 1 ;;\nesac"
            ),
        );
        let out = dir.display();
        let claude = script(
            dir,
            "claude",
            &format!(
                "echo \"$@\" > '{out}/argv'\nprintf %s \"$ANTHROPIC_API_KEY\" > '{out}/env'\necho \"$ANTHROPIC_API_KEY\" >&2\necho '{answer}'\nexit {code}"
            ),
        );
        Tools { secret, claude }
    }

    /// The store's whole file: its tables and its op log.
    fn stored(dir: &Path) -> Vec<u8> {
        let mut all = vec![];
        for name in ["tasks.sqlite", "tasks.sqlite-wal"] {
            all.extend(std::fs::read(dir.join(name)).unwrap_or_default());
        }
        all
    }

    fn holds_value(bytes: &[u8]) -> bool {
        bytes.windows(VALUE.len()).any(|w| w == VALUE.as_bytes())
    }

    #[dash_test]
    fn key_add_stores_a_row_naming_a_secret_and_key_rm_removes_it() {
        let dir = TestDir::new("keys-add-rm");
        let tools = tools(&dir.0, "{}", 0);
        let mut keys = keys::open(&dir.0).expect("open");
        let row = added(&mut keys, &tools, "anthropic", "anthropic", Some(0.0)).expect("add");
        assert_eq!(
            row,
            "anthropic  secret anthropic  spent $0.0000 of a $0.00 cap"
        );
        let listed = keys::open(&dir.0).expect("reopen").list().expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(
            (listed[0].id.as_str(), listed[0].secret.as_str()),
            ("anthropic", "anthropic")
        );
        assert!(keys.remove("anthropic").expect("rm"));
        assert!(
            keys::open(&dir.0)
                .expect("reopen")
                .list()
                .expect("list")
                .is_empty()
        );
        let other = added(&mut keys, &tools, "anthropic", "malt", None);
        assert!(other.is_err(), "a name holding no ANTHROPIC_API_KEY");
        assert!(
            added(&mut keys, &tools, "gemini", "anthropic", None).is_err(),
            "no such kind"
        );
        assert!(
            added(&mut keys, &tools, "anthropic", "anthropic", Some(-1.0)).is_err(),
            "no negative cap"
        );
    }

    #[dash_test]
    fn a_key_value_in_place_of_a_name_is_refused_unrepeated_and_the_store_unchanged() {
        let dir = TestDir::new("keys-value");
        let tools = tools(&dir.0, "{}", 0);
        let mut keys = keys::open(&dir.0).expect("open");
        let before = stored(&dir.0);
        let refused = added(&mut keys, &tools, "anthropic", VALUE, Some(5.0)).expect_err("refused");
        assert!(!refused.contains(VALUE), "{refused}");
        assert!(refused.contains("never a key's value"), "{refused}");
        assert!(keys.list().expect("list").is_empty());
        assert_eq!(stored(&dir.0), before, "not one byte written");
    }

    #[dash_test]
    fn a_run_gets_its_secret_in_its_environment_only_and_counts_its_spend() {
        let dir = TestDir::new("keys-run");
        let answer = r#"{"type":"result","result":"hello","total_cost_usd":0.25}"#;
        let tools = tools(&dir.0, answer, 0);
        let mut keys = keys::open(&dir.0).expect("open");
        added(&mut keys, &tools, "anthropic", "anthropic", Some(0.0)).expect("add");
        let refused = ran(&mut keys, &tools, &dir.0, "anthropic", "say hello").expect_err("no cap");
        assert!(refused.contains("cap"), "{refused}");
        assert!(
            !dir.0.join("argv").exists(),
            "a row with no room runs nothing"
        );
        added(&mut keys, &tools, "anthropic", "anthropic", Some(0.5)).expect("cap");
        let said = ran(&mut keys, &tools, &dir.0, "anthropic", "say hello").expect("run");
        assert_eq!(said, "hello\nspent: $0.2500");
        let env = std::fs::read_to_string(dir.0.join("env")).expect("env");
        assert_eq!(env, VALUE, "in its environment");
        let argv = std::fs::read_to_string(dir.0.join("argv")).expect("argv");
        assert_eq!(
            argv.trim(),
            "--tools  --max-budget-usd 0.5000 --output-format json -p say hello",
            "never on its command line; no tools, the rest of its cap its budget"
        );
        assert!(
            !holds_value(&stored(&dir.0)),
            "never in the store or its op log"
        );
        let key = &keys.list().expect("list")[0];
        assert_eq!(key.spent, 0.25);
        assert_eq!(key.limit().week, 50.0);
        assert!(
            ran(&mut keys, &tools, &dir.0, "anthropic", "--version").is_err(),
            "no flag"
        );
    }

    #[dash_test]
    fn a_failed_run_counts_what_it_spent_and_never_says_the_key() {
        let dir = TestDir::new("keys-failed");
        let answer = r#"{"type":"result","is_error":true,"result":"no","total_cost_usd":0.1}"#;
        let tools = tools(&dir.0, answer, 1);
        let mut keys = keys::open(&dir.0).expect("open");
        added(&mut keys, &tools, "anthropic", "anthropic", Some(1.0)).expect("add");
        let failed = ran(&mut keys, &tools, &dir.0, "anthropic", "x").expect_err("failed");
        assert!(
            failed.contains("spent: $0.1000") && failed.contains("[key]"),
            "{failed}"
        );
        assert!(!failed.contains(VALUE), "{failed}");
        assert_eq!(keys.list().expect("list")[0].spent, 0.1);
        assert!(!holds_value(&stored(&dir.0)));
    }

    #[dash_test]
    fn a_tool_that_never_started_charges_nothing_and_never_says_the_key() {
        let dir = TestDir::new("keys-no-tool");
        let mut tools = tools(&dir.0, "{}", 0);
        tools.claude = dir.0.join("not-there").display().to_string();
        let mut keys = keys::open(&dir.0).expect("open");
        added(&mut keys, &tools, "anthropic", "anthropic", Some(1.0)).expect("add");
        let failed = ran(&mut keys, &tools, &dir.0, "anthropic", "x").expect_err("no tool");
        assert!(
            failed.contains("not-there") && !failed.contains(VALUE),
            "{failed}"
        );
        assert_eq!(keys.list().expect("list")[0].spent, 0.0);
    }

    #[dash_test]
    fn a_second_run_at_once_is_refused_and_a_secret_tool_s_failure_never_echoes_its_output() {
        let dir = TestDir::new("keys-lock");
        let mut tools = tools(&dir.0, "{}", 0);
        let mut keys = keys::open(&dir.0).expect("open");
        added(&mut keys, &tools, "anthropic", "anthropic", Some(1.0)).expect("add");
        let held = File::create(dir.0.join("key-anthropic.lock")).expect("lock");
        held.try_lock().expect("held");
        let refused = ran(&mut keys, &tools, &dir.0, "anthropic", "x").expect_err("running");
        assert!(refused.contains("running"), "{refused}");
        drop(held);
        tools.secret = script(&dir.0, "secret-fails", &format!("echo '{VALUE}'\nexit 3"));
        let failed = ran(&mut keys, &tools, &dir.0, "anthropic", "x").expect_err("get failed");
        assert!(
            failed.contains("exit status: 3") && !failed.contains(VALUE),
            "{failed}"
        );
        assert!(!dir.0.join("argv").exists(), "nothing ran");
    }

    #[dash_test]
    fn key_add_with_no_cap_keeps_the_row_s_own() {
        let dir = TestDir::new("keys-keep-cap");
        let tools = tools(&dir.0, "{}", 0);
        let mut keys = keys::open(&dir.0).expect("open");
        added(&mut keys, &tools, "anthropic", "anthropic", Some(10.0)).expect("add");
        added(&mut keys, &tools, "anthropic", "anthropic", None).expect("again");
        assert_eq!(keys.list().expect("list")[0].cap, 10.0);
    }

    #[dash_test]
    fn a_run_whose_cost_cannot_be_read_takes_the_rest_of_the_cap() {
        let dir = TestDir::new("keys-unknown");
        let tools = tools(&dir.0, "not json", 0);
        let mut keys = keys::open(&dir.0).expect("open");
        added(&mut keys, &tools, "anthropic", "anthropic", Some(1.0)).expect("add");
        let said = ran(&mut keys, &tools, &dir.0, "anthropic", "x").expect("ran");
        assert!(said.contains("rest of its cap"), "{said}");
        let key = &keys.list().expect("list")[0];
        assert!(!key.limit().room(), "{key:?}");
        assert!(
            ran(&mut keys, &tools, &dir.0, "anthropic", "x").is_err(),
            "no second run"
        );
    }

    #[dash_test]
    fn the_policy_snapshot_holds_a_key_s_spend_against_its_cap() {
        use dash_core::tasks::{Agent, Pick, Policy, Queue, Queued, plan};
        let dir = TestDir::new("keys-snapshot");
        let mut keys = keys::open(&dir.0).expect("open");
        keys.add("anthropic", "anthropic", 4.0).expect("add");
        keys.spend("r1", "anthropic", 1.0).expect("spend");
        // The picker's agent as the binary wires it: Claude's accounts read from a status file, no Codex, the key rows.
        let readings = TestDir::new("keys-snapshot-claude");
        let reading = serde_json::json!({"config_dir": "/h/.claude/accounts/second", "rate_limits": {
            "seven_day": {"used_percentage": 10, "resets_at": u64::MAX}}});
        std::fs::write(readings.0.join("s.json"), reading.to_string()).expect("reading");
        let claude = dash_agent::ClaudeCode {
            projects: PathBuf::new(),
            limits: readings.0.clone(),
        };
        let bin = readings.0.join("no-codex").display().to_string();
        let codex = dash_agent::codex::Codex {
            bin,
            sessions: PathBuf::new(),
        };
        let agent = crate::kinds::Agents::new(claude, codex, &dir.0);
        // The policy the orchestrator's `next` hands its snapshot, as `plan` builds it.
        struct Seen(std::cell::RefCell<Vec<Limit>>);
        impl Policy for Seen {
            fn pick(&self, snapshot: &dash_core::tasks::Snapshot) -> Pick {
                *self.0.borrow_mut() = snapshot.limits.clone();
                Pick::Wait("seen".into())
            }
            fn after(
                &self,
                _: &dash_core::tasks::Verdict,
                _: u32,
                _: bool,
            ) -> dash_core::tasks::Step {
                dash_core::tasks::Step::Keep
            }
            fn retry(&self, _: u32) -> bool {
                false
            }
        }
        let seen = Seen(Default::default());
        let task = Queued {
            held: None,
            id: "a".into(),
            title: String::new(),
            reopened_ms: None,
            waits: vec![],
            layer: None,
            builds_nothing: false,
            rank: Default::default(),
            free: None,
            demand: Default::default(),
            route: None,
            ..Default::default()
        };
        let queue = Queue {
            seats: 5,
            seats_why: String::new(),
            tasks: vec![task],
        };
        plan(&seen, queue, &[], 0, agent.limits().expect("limits"), None);
        let limits = seen.0.borrow();
        assert_eq!(limits.len(), 2, "{limits:?}");
        assert_eq!(
            (limits[0].account.as_str(), limits[0].key),
            ("second", false)
        );
        let key = &limits[1];
        assert_eq!(
            (key.account.as_str(), key.week, key.key),
            ("anthropic", 25.0, true)
        );
    }
}
