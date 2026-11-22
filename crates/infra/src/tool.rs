//! `dash infra`: OpenTofu, wrapped (rfc-iac; @nha, 2026-11-18: « Decided: OpenTofu is the engine, wrapped by our Rust
//! layer. -> agreed. »). tofu plans and keeps the state; this layer runs it, reads its JSON, and answers one versioned
//! contract that core/infra (RFC-0024, through `spawn_process`) and an agent in a shell read the same way:
//! - stdout holds one JSON document, `"contract": "infra/1"`, and nothing else;
//! - exit 0 no change, 2 changes planned (tofu's -detailed-exitcode), 3 the guard refused the plan (its refusals in the
//!   document), 1 an error (its words on stderr, nothing on stdout): never a green on a failure. Exit 0, 2 and 3 always
//!   come with the document: a caller reads `"contract"` first, since dash's usage also exits 2 with nothing on stdout;
//! - no attribute value leaves except on an allow-list, never user_data, cloud-init or anything tofu marks sensitive;
//!   tofu's stdout never, its stderr only through [`said`].
//!
//! The guard is `infra/guard.json`, one for every unit, reviewed with the PRs that change it; a unit is a folder under
//! `infra/` and may not bring a guard of its own. It holds each unit's protected addresses (no destroy, no replace, no
//! forget, under their previous address too), the resource types allowed, each priced by its size (Hostman's preset),
//! and the credit's end: a plan that grows the month is held against the balance hostman-ci's job last read, within 24
//! hours (rfc-iac S4).
//! oxid, reviewed and not adopted, taught the rules: private/docs/oxid-review/.
//! No apply here: rfc-iac's apply runs from main, after this plan's guard, in a later slice.
//!
//! terraform-wrapper 0.4.0 (@nha, 2026-11-18: « https://docs.rs/terraform-wrapper/latest/terraform_wrapper/ might be
//! interesting. »), read 2026-11-18 and not taken: its `ResourceChange` has no `change.importing` (the import count),
//! its state `Resource` no `depends_on` nor `index` (the graph's edges), its `StateRepresentation` requires `values`, so
//! the `{"format_version": "1.0"}` tofu prints for an empty state fails to parse, and its timeout drops `cmd.output()`
//! without `kill_on_drop`, leaving tofu running. It would also bring tokio, which, thiserror and tracing for a few
//! fields. tofu runs through the host's `run` (its group killed at the limit) and its JSON is read as
//! `serde_json::Value`, the fields named where they are read. Revisit if it gains those fields.

use serde_json::{Map, Value, json};
use std::path::Path;

pub const CONTRACT: &str = "infra/1";
/// The guard's file, under the checkout's `infra/`.
pub const GUARD: &str = "guard.json";
/// Exit codes, the contract's: 0 no change, 2 changes, 3 refused by the guard, 1 an error.
pub const CHANGES: i32 = 2;
pub const REFUSED: i32 = 3;
pub const ERROR: i32 = 1;
/// How old a balance reading may be for a plan that grows the month.
pub const BALANCE_MS: i64 = 86_400_000;

/// The attributes `state` and `graph` copy from the state (RFC-0024 section 4.6): identity, place, addresses, ports,
/// sizes. Anything else stays out, user_data, cloud-init and passwords first.
pub const ATTRIBUTES: &[&str] = &[
    "id",
    "name",
    "type",
    "region",
    "zone",
    "availability_zone",
    "location",
    "tags",
    "labels",
    "ip",
    "ips",
    "ipv4",
    "ipv6",
    "port",
    "ports",
    "size",
    "preset_id",
    "os_id",
    "status",
];

/// `infra/guard.json`, read.
#[derive(Debug, Clone, PartialEq)]
pub struct Guard {
    pub currency: String,
    /// The day the credit ends, 2026-12-23.
    pub expires: String,
    /// Each type allowed: `{"monthly": n}`, or `{"size": attribute, "monthly": {size: n}}`.
    pub types: Map<String, Value>,
    /// Each unit's protected addresses, by its folder under infra/.
    pub protected: Map<String, Value>,
    /// Each provider folder's secrets handed to tofu: `{"HM_TOKEN": "hostman HOSTMAN_API_KEY"}`, bin/secret's words.
    pub secrets: Map<String, Value>,
}

impl Guard {
    pub fn parse(text: &str) -> Result<Guard, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("{GUARD}: {e}"))?;
        let expires = v["expires"]
            .as_str()
            .ok_or(format!("{GUARD}: no expires"))?;
        dash_sdk::days(expires).ok_or(format!("{GUARD}: expires {expires} is not a day"))?;
        let map = |key: &str| {
            v[key]
                .as_object()
                .cloned()
                .ok_or(format!("{GUARD}: no \"{key}\" map"))
        };
        Ok(Guard {
            currency: v["currency"]
                .as_str()
                .ok_or(format!("{GUARD}: no currency"))?
                .to_string(),
            expires: expires.to_string(),
            types: map("types")?,
            protected: map("protected")?,
            secrets: v["secrets"].as_object().cloned().unwrap_or_default(),
        })
    }

    /// A resource's month by its type and size, read from its `state` (before or after the change).
    pub fn month(&self, kind: &str, state: &Value) -> Result<f64, String> {
        let rule = self
            .types
            .get(kind)
            .ok_or(format!("type {kind} is not on the guard's list"))?;
        let Some(size) = rule["size"].as_str() else {
            return rule["monthly"]
                .as_f64()
                .ok_or(format!("no month for {kind} in {GUARD}"));
        };
        let value = match &state[size] {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            _ => return Err(format!("{kind}: its {size} unknown, so its month is")),
        };
        rule["monthly"][&value]
            .as_f64()
            .ok_or(format!("no month for {kind} {size} {value} in {GUARD}"))
    }
}

/// The balance hostman-ci's job last read from Hostman (its own database), in euros by his word (@nha, 2026-11-18:
/// « euros. the API is wrong. »), and when.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Balance {
    pub amount: f64,
    pub at_ms: i64,
}

/// One planned change: its values stay in memory, for the guard's prices; the document names none.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub address: String,
    /// tofu's `previous_address`: a `moved` block's old address.
    pub previous: Option<String>,
    pub mode: String,
    pub kind: String,
    /// create, update, delete, replace, no-op, read or forget
    pub action: String,
    pub importing: Option<String>,
    pub before: Value,
    pub after: Value,
}

/// The changes of `tofu show -json <plan>`; a plan that says it errored, or an action this layer does not know, is an
/// error, never read as no change.
pub fn changes(plan: &Value) -> Result<Vec<Change>, String> {
    if plan["errored"].as_bool() == Some(true) {
        return Err("tofu's plan says it errored".into());
    }
    let list = plan["resource_changes"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    list.iter()
        .map(|c| {
            let actions: Vec<&str> = c["change"]["actions"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let action = match actions.as_slice() {
                [one @ ("create" | "update" | "delete" | "no-op" | "read" | "forget")] => *one,
                ["delete", "create"] | ["create", "delete"] => "replace",
                other => {
                    return Err(format!(
                        "{}: actions {other:?} not known to dash infra",
                        c["address"]
                    ));
                }
            };
            let text = |v: &Value| v.as_str().map(String::from);
            Ok(Change {
                address: text(&c["address"]).unwrap_or_default(),
                previous: text(&c["previous_address"]),
                mode: text(&c["mode"]).unwrap_or_default(),
                kind: text(&c["type"]).unwrap_or_default(),
                action: action.to_string(),
                importing: text(&c["change"]["importing"]["id"]),
                before: c["change"]["before"].clone(),
                after: c["change"]["after"].clone(),
            })
        })
        .collect()
}

/// tofu's counts (« Plan: 2 to import, 0 to add, 0 to change, 0 to destroy. »): a replace is one to add and one to
/// destroy, also counted apart.
pub fn counts(changes: &[Change]) -> Value {
    let n = |f: &dyn Fn(&Change) -> bool| changes.iter().filter(|c| f(c)).count();
    json!({
        "import": n(&|c| c.importing.is_some()),
        "add": n(&|c| matches!(c.action.as_str(), "create" | "replace")),
        "change": n(&|c| c.action == "update"),
        "destroy": n(&|c| matches!(c.action.as_str(), "delete" | "replace")),
        "replace": n(&|c| c.action == "replace"),
        "forget": n(&|c| c.action == "forget"),
    })
}

/// The address without its instance keys, in every segment: hm_server.ci[0] is hm_server.ci, module.net[0].hm_floating_ip.ci
/// is module.net.hm_floating_ip.ci. A key's own brackets and dots sit inside its quotes.
pub fn base(address: &str) -> String {
    let (mut out, mut depth, mut quoted) = (String::new(), 0, false);
    for c in address.chars() {
        match c {
            '"' if depth > 0 => quoted = !quoted,
            '[' if !quoted => depth += 1,
            ']' if !quoted && depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// Whether tofu's `sensitive_values` marks any part of an attribute: `true` anywhere in it. A collection's `{}` or
/// `[false]` marks nothing.
fn marked(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(Value::Array(items)) => items.iter().any(|i| marked(Some(i))),
        Some(Value::Object(map)) => map.values().any(|i| marked(Some(i))),
        _ => false,
    }
}

/// The guard's verdict on a plan of `unit` (its folder under infra/): its refusals, and the month before and after.
pub fn guard(
    changes: &[Change],
    rules: &Guard,
    unit: &str,
    balance: Option<Balance>,
    now_ms: i64,
) -> (Vec<String>, Value) {
    let mut refusals = vec![];
    let protected: Vec<&str> = rules
        .protected
        .get(unit)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let (mut before, mut after) = (0.0, 0.0);
    for c in changes.iter().filter(|c| c.mode == "managed") {
        let held = |a: &String| Some(base(a)).filter(|a| protected.contains(&a.as_str()));
        // destroyed, replaced or forgotten under either address, or moved away from a protected one whatever the action:
        // a move takes it out of protection, and the next PR deletes it unguarded (/code-review)
        let gone = matches!(c.action.as_str(), "delete" | "replace" | "forget");
        let found = match (gone, c.previous.as_ref().and_then(held)) {
            (_, Some(p)) => Some(p),
            (true, None) => held(&c.address),
            (false, None) => None,
        };
        if let Some(p) = found {
            refusals.push(format!(
                "{p} is protected: the plan would {} {}",
                c.action, c.address
            ));
        }
        // a forget leaves the state, not the bill: before and after both hold it at its old size
        let (was, will) = match c.action.as_str() {
            "create" => (None, Some(&c.after)),
            "delete" => (Some(&c.before), None),
            "forget" => (Some(&c.before), Some(&c.before)),
            "no-op" if c.before.is_null() => (Some(&c.after), Some(&c.after)),
            _ => (Some(&c.before), Some(&c.after)),
        };
        for (state, sum) in [(was, &mut before), (will, &mut after)] {
            match state.map(|s| rules.month(&c.kind, s)) {
                Some(Ok(m)) => *sum += m,
                Some(Err(why)) => refusals.push(format!("{}: {why}", c.address)),
                None => {}
            }
        }
    }
    refusals.dedup();
    let days_left = dash_sdk::days(&rules.expires).unwrap_or(0) - now_ms.div_euclid(86_400_000);
    let to_expiry = after * days_left.max(0) as f64 / 30.0;
    let cur = &rules.currency;
    if after > before {
        let fresh = balance.filter(|b| now_ms - b.at_ms <= BALANCE_MS);
        match fresh {
            _ if days_left <= 0 => refusals.push(format!("the credit ended {}: no spend may grow", rules.expires)),
            None => refusals.push(
                "the month grows and no Hostman balance read in the last 24 hours: hostman-ci's job records it".into(),
            ),
            Some(b) if to_expiry > b.amount => refusals.push(format!(
                "the month grows from {before:.2} to {after:.2} {cur}: {to_expiry:.2} to {}, past the {:.2} {cur} left",
                rules.expires, b.amount
            )),
            Some(_) => {}
        }
    }
    let spend = json!({
        "monthly_before": before,
        "monthly_after": after,
        "to_expiry": (to_expiry * 100.0).round() / 100.0,
        "days_left": days_left,
        "balance": balance.map(|b| json!({"amount": b.amount, "at_ms": b.at_ms})),
        "currency": cur,
        "expires": rules.expires,
    });
    (refusals, spend)
}

/// tofu's stderr as an error may say it: its diagnostics' words, never a quoted source line (« 3:   root_pass = ... »)
/// nor an assignment, so no value written in the files reaches a caller's log. tofu's stdout is never said.
pub fn said(stderr: &str) -> String {
    let quoted = |l: &str| {
        l.trim_start()
            .split_once(':')
            .is_some_and(|(n, _)| n.parse::<u32>().is_ok())
    };
    // the values tofu prints under a snippet: « │ local.ci is "..." », its frame « ├────», « ╷ », « ╵ »
    let framed = |l: &str| l.contains(['│', '├', '╷', '╵']) || l.contains(" is \"");
    let kept: Vec<&str> = stderr
        .lines()
        .filter(|l| !quoted(l) && !framed(l) && !l.contains(" = "))
        .take(40)
        .collect();
    kept.join("\n").trim().to_string()
}

/// Every resource of `tofu show -json` (the state), child modules included: address, base address (what an edge's
/// `depends_on` names), mode, type, provider, index, the addresses it depends on, and its attributes on the allow-list,
/// each one tofu marks sensitive left out.
pub fn resources(state: &Value) -> Vec<Value> {
    let mut out = vec![];
    let mut modules = vec![&state["values"]["root_module"]];
    while let Some(module) = modules.pop() {
        for r in module["resources"].as_array().into_iter().flatten() {
            let sensitive = &r["sensitive_values"];
            let attributes: Map<String, Value> = r["values"]
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(k, v)| {
                    ATTRIBUTES.contains(&k.as_str())
                        && !v.is_null()
                        && !marked(sensitive.get(k.as_str()))
                })
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            out.push(json!({
                "address": r["address"],
                "base": base(r["address"].as_str().unwrap_or_default()),
                "mode": r["mode"],
                "type": r["type"],
                "provider": r["provider_name"],
                "index": r.get("index").cloned().unwrap_or(Value::Null),
                "depends_on": r.get("depends_on").cloned().unwrap_or(json!([])),
                "attributes": attributes,
            }));
        }
        modules.extend(module["child_modules"].as_array().into_iter().flatten());
    }
    out.sort_by(|a, b| a["address"].as_str().cmp(&b["address"].as_str()));
    out
}

/// The map RFC-0024's core/infra builds its nodes and typed links from: each resource a node, each `depends_on` an edge
/// from the resource to what it depends on (a base address); core/infra turns (type, type) into routes_to, contains or
/// depends_on.
pub fn graph(state: &Value) -> Value {
    let nodes = resources(state);
    let edges: Vec<Value> = nodes
        .iter()
        .flat_map(|n| {
            n["depends_on"]
                .as_array()
                .into_iter()
                .flatten()
                .map(move |to| json!({ "from": n["address"], "to": to, "kind": "depends_on" }))
        })
        .collect();
    let nodes: Vec<Value> = nodes
        .into_iter()
        .map(|mut n| {
            n.as_object_mut().map(|m| m.remove("depends_on"));
            n
        })
        .collect();
    json!({ "nodes": nodes, "edges": edges })
}

/// tofu in a unit's folder, its arguments and limit in seconds: its exit code, stdout and stderr. The plugin's root
/// hands it in (lib.rs, through the host's `run`), so this tool takes the SDK alone.
pub type Tofu<'a> = &'a dyn Fn(&Path, &[&str], u64) -> Result<(i32, String, String), String>;

fn tofu_ok(tofu: Tofu, dir: &Path, args: &[&str], limit: u64) -> Result<String, String> {
    match tofu(dir, args, limit)? {
        (0, out, _) => Ok(out),
        (code, _, err) => Err(format!("tofu {}: exit {code}\n{}", args[0], said(&err))),
    }
}

/// tofu init, the lock file read only: a provider the lock does not hold for this platform is an error, never a
/// rewritten tracked file; `tofu providers lock -platform=...` adds one.
pub fn init(tofu: Tofu, dir: &Path) -> Result<(), String> {
    let args = ["init", "-input=false", "-no-color", "-lockfile=readonly"];
    tofu_ok(tofu, dir, &args, 600).map(|_| ())
}

/// The saved plan, every value in clear: removed on every way out of `plan`; the root removes it on a signal too.
struct Saved<'a>(&'a Path);

impl Drop for Saved<'_> {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.0);
    }
}

/// `dash infra plan` on the unit in `dir` (`unit` its folder under infra/), the plan saved at `saved`: the document and
/// its exit code.
pub fn plan(
    tofu: Tofu,
    dir: &Path,
    unit: &str,
    rules: &Guard,
    balance: Option<Balance>,
    now_ms: i64,
    saved: &Path,
) -> Result<(Value, i32), String> {
    init(tofu, dir)?;
    let _saved = Saved(saved);
    let out = format!("-out={}", saved.display());
    let args = [
        "plan",
        "-input=false",
        "-no-color",
        "-detailed-exitcode",
        "-lock-timeout=60s",
        out.as_str(),
    ];
    // tofu's own plan text is not echoed: it prints every attribute value, cloud-init and user_data with them
    let (code, _, err) = tofu(dir, &args, 900)?;
    if code != 0 && code != CHANGES {
        return Err(format!("tofu plan: exit {code}\n{}", said(&err)));
    }
    let shown = tofu_ok(
        tofu,
        dir,
        &["show", "-json", "-no-color", &saved.display().to_string()],
        300,
    )?;
    let shown: Value = serde_json::from_str(&shown).map_err(|e| format!("tofu show -json: {e}"))?;
    let changes = changes(&shown)?;
    let (refusals, spend) = guard(&changes, rules, unit, balance, now_ms);
    let exit = if !refusals.is_empty() { REFUSED } else { code };
    let doc = json!({
        "contract": CONTRACT,
        // the version the plan says wrote it: no second tofu started for it (/code-review)
        "engine": { "name": "tofu", "version": shown["terraform_version"] },
        "unit": unit,
        "counts": counts(&changes),
        "changes": changes.iter().map(|c| json!({
            "address": c.address, "previous_address": c.previous, "mode": c.mode, "type": c.kind,
            "action": c.action, "importing": c.importing,
        })).collect::<Vec<_>>(),
        "guard": { "passed": refusals.is_empty(), "refusals": refusals, "spend": spend },
    });
    Ok((doc, exit))
}

/// `tofu show -json` of the unit's state.
pub fn shown_state(tofu: Tofu, dir: &Path) -> Result<Value, String> {
    init(tofu, dir)?;
    let shown = tofu_ok(tofu, dir, &["show", "-json", "-no-color"], 300)?;
    serde_json::from_str(&shown).map_err(|e| format!("tofu show -json: {e}"))
}
