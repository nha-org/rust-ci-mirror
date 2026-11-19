use crate::tool::*;
use crate::{balance, run as cli};
use dash_sdk::dash_meter::TestDir;
use dash_sdk::{
    FieldWrite, Host, MemoryLog, OpLog, Program, Ran, StandIn, WallClock, dash_meter, dash_test,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// The central guard of the tests, as infra/guard.json: Hostman's server priced by its preset (6273, ci's, 20 a month;
/// 6271 at 8), its IPv4 at 2, a key free; ci and its IPv4 protected in the unit hostman/ci.
const HOSTMAN_GUARD: &str = r#"{
  "currency": "EUR",
  "expires": "2026-12-23",
  "types": {
    "hm_server": {"size": "preset_id", "monthly": {"6273": 20, "6271": 8}},
    "hm_floating_ip": {"monthly": 2},
    "hm_ssh_key": {"monthly": 0}
  },
  "protected": {"hostman/ci": ["hm_server.ci", "hm_floating_ip.ci"]}
}"#;

const DAY_MS: i64 = 86_400_000;

fn ms(d: &str) -> i64 {
    dash_sdk::days(d).unwrap() * DAY_MS
}

/// A change as tofu's plan holds it: a server's preset before and after, by its action.
fn change(address: &str, action: &str, before: Value, after: Value) -> Change {
    Change {
        address: address.into(),
        previous: None,
        mode: "managed".into(),
        kind: address.split('.').next().unwrap().into(),
        action: action.into(),
        importing: None,
        before,
        after,
    }
}

fn server(preset: i64) -> Value {
    json!({"name": "ci", "preset_id": preset})
}

/// ci and its IPv4, as they stand: 22 a month.
fn kept() -> Vec<Change> {
    vec![
        change("hm_server.ci", "no-op", server(6273), server(6273)),
        change("hm_floating_ip.ci", "no-op", json!({}), json!({})),
    ]
}

/// tofu's plan JSON, cut to what the layer reads: ci and its IPv4 imported with nothing to change (rfc-iac S1's goal),
/// a replace, an update, a data source read, a move.
#[dash_test]
fn changes_and_counts_read_tofus_plan_as_tofu_counts_it() {
    let plan = json!({"format_version": "1.2", "errored": false, "resource_changes": [
        {"address": "hm_server.ci", "mode": "managed", "type": "hm_server",
         "change": {"actions": ["no-op"], "importing": {"id": "9608429"}, "before": server(6273), "after": server(6273)}},
        {"address": "hm_floating_ip.ci", "mode": "managed", "type": "hm_floating_ip",
         "change": {"actions": ["no-op"], "importing": {"id": "ba7d3127"}}}
    ]});
    let found = changes(&plan).unwrap();
    assert_eq!(found[0].importing.as_deref(), Some("9608429"));
    assert_eq!(found[0].after, server(6273));
    assert_eq!(
        counts(&found),
        json!({"import": 2, "add": 0, "change": 0, "destroy": 0, "replace": 0, "forget": 0})
    );

    let plan = json!({"resource_changes": [
        {"address": "hm_server.tmp", "mode": "managed", "type": "hm_server", "change": {"actions": ["delete", "create"]}},
        {"address": "hm_server.web", "previous_address": "hm_server.old", "mode": "managed", "type": "hm_server",
         "change": {"actions": ["update"]}},
        {"address": "data.hm_os.ubuntu", "mode": "data", "type": "hm_os", "change": {"actions": ["read"]}}
    ]});
    let found = changes(&plan).unwrap();
    assert_eq!(found[0].action, "replace");
    assert_eq!(found[1].previous.as_deref(), Some("hm_server.old"));
    assert_eq!(
        counts(&found),
        json!({"import": 0, "add": 1, "change": 1, "destroy": 1, "replace": 1, "forget": 0})
    );

    assert!(
        changes(&json!({"errored": true})).is_err(),
        "an errored plan is never no change"
    );
    let odd =
        json!({"resource_changes": [{"address": "x.y", "change": {"actions": ["teleport"]}}]});
    assert!(
        changes(&odd).unwrap_err().contains("teleport"),
        "an action not known is an error"
    );
}

/// Point 1 and 2 of the PR's review: the guard is the central one; a protected address is protected under its previous
/// address too, and a forget of it (its block removed) is refused as a destroy is.
#[dash_test]
fn the_guard_protects_by_unit_previous_address_and_forget() {
    let rules = Guard::parse(HOSTMAN_GUARD).unwrap();
    let now = ms("2026-11-18");
    let fresh = Some(Balance {
        amount: 27.16,
        at_ms: now,
    });
    for action in ["delete", "replace", "forget"] {
        let c = change("hm_server.ci", action, server(6273), server(6273));
        let (refusals, _) = guard(&[c], &rules, "hostman/ci", fresh, now);
        assert!(
            refusals
                .iter()
                .any(|r| r.contains("hm_server.ci is protected")),
            "{action}: {refusals:?}"
        );
    }
    let moved = Change {
        previous: Some("hm_server.ci".into()),
        ..change("hm_server.renamed", "delete", server(6273), Value::Null)
    };
    let (refusals, _) = guard(&[moved], &rules, "hostman/ci", fresh, now);
    assert!(
        refusals
            .iter()
            .any(|r| r.contains("hm_server.ci is protected")),
        "{refusals:?}"
    );
    let ip = Change {
        kind: "hm_floating_ip".into(),
        ..change(
            "module.net[0].hm_floating_ip.ci",
            "delete",
            json!({}),
            Value::Null,
        )
    };
    let (refusals, _) = guard(&[ip], &rules, "hostman/ci", fresh, now);
    assert!(
        refusals.is_empty(),
        "module.net.hm_floating_ip.ci is not protected: {refusals:?}"
    );
    assert_eq!(
        base("module.net[0].hm_floating_ip.ci"),
        "module.net.hm_floating_ip.ci"
    );
    // the same address in another unit is that unit's to protect
    let other = change("hm_server.ci", "delete", server(6273), Value::Null);
    let (refusals, _) = guard(&[other], &rules, "hostman/tmp", fresh, now);
    assert!(refusals.is_empty(), "{refusals:?}");
}

/// Point 3: a price by type and size. An unknown size or a type off the list is refused; an update that changes the
/// size is priced before and after.
#[dash_test]
fn the_guard_prices_by_type_and_size() {
    let rules = Guard::parse(HOSTMAN_GUARD).unwrap();
    let now = ms("2026-11-18");
    let fresh = Some(Balance {
        amount: 27.16,
        at_ms: now,
    });
    let (refusals, spend) = guard(&kept(), &rules, "hostman/ci", fresh, now);
    assert!(refusals.is_empty(), "{refusals:?}");
    assert_eq!(
        (
            spend["monthly_before"].clone(),
            spend["monthly_after"].clone()
        ),
        (json!(22.0), json!(22.0))
    );

    let down = vec![
        change("hm_server.ci", "update", server(6273), server(6271)),
        kept()[1].clone(),
    ];
    let (refusals, spend) = guard(&down, &rules, "hostman/ci", fresh, now);
    assert!(refusals.is_empty(), "{refusals:?}");
    assert_eq!(
        (
            spend["monthly_before"].clone(),
            spend["monthly_after"].clone()
        ),
        (json!(22.0), json!(10.0))
    );

    let odd = vec![change("hm_server.big", "create", Value::Null, server(9999))];
    let (refusals, _) = guard(&odd, &rules, "hostman/ci", fresh, now);
    assert!(
        refusals
            .iter()
            .any(|r| r.contains("no month for hm_server preset_id 9999")),
        "{refusals:?}"
    );
    let unknown = vec![change(
        "hm_server.big",
        "create",
        Value::Null,
        json!({"name": "big"}),
    )];
    let (refusals, _) = guard(&unknown, &rules, "hostman/ci", fresh, now);
    assert!(
        refusals.iter().any(|r| r.contains("preset_id unknown")),
        "{refusals:?}"
    );
    let off = vec![change("hm_database.main", "create", Value::Null, json!({}))];
    let (refusals, _) = guard(&off, &rules, "hostman/ci", fresh, now);
    assert!(
        refusals
            .iter()
            .any(|r| r.contains("not on the guard's list")),
        "{refusals:?}"
    );
    assert!(
        Guard::parse(r#"{"types": {}}"#).is_err(),
        "a guard missing its rules is refused"
    );
}

/// Point 4: the credit is the balance hostman-ci's job last read, never a local estimate. A plan that grows the month
/// needs a reading of the last 24 hours, and its month to the credit's end within that balance; one that grows nothing
/// needs none.
#[dash_test]
fn the_guard_holds_a_growing_month_against_the_balance_read() {
    let rules = Guard::parse(HOSTMAN_GUARD).unwrap();
    let now = ms("2026-12-20");
    let grown = vec![
        kept()[0].clone(),
        kept()[1].clone(),
        change("hm_floating_ip.ci2", "create", Value::Null, json!({})),
    ];
    // 24 a month, 3 days left: 2.40
    let (refusals, spend) = guard(
        &grown,
        &rules,
        "hostman/ci",
        Some(Balance {
            amount: 3.0,
            at_ms: now,
        }),
        now,
    );
    assert!(refusals.is_empty(), "{refusals:?}");
    assert_eq!(spend["to_expiry"], 2.4);
    let (refusals, _) = guard(
        &grown,
        &rules,
        "hostman/ci",
        Some(Balance {
            amount: 2.0,
            at_ms: now,
        }),
        now,
    );
    assert!(
        refusals.iter().any(|r| r.contains("past the 2.00 EUR")),
        "{refusals:?}"
    );
    let old = Some(Balance {
        amount: 27.16,
        at_ms: now - DAY_MS - 1,
    });
    for read in [old, None] {
        let (refusals, _) = guard(&grown, &rules, "hostman/ci", read, now);
        assert!(
            refusals
                .iter()
                .any(|r| r.contains("no Hostman balance read in the last 24 hours")),
            "{refusals:?}"
        );
    }
    let (refusals, _) = guard(&kept(), &rules, "hostman/ci", None, now);
    assert!(
        refusals.is_empty(),
        "nothing grows, no reading needed: {refusals:?}"
    );
    let (refusals, _) = guard(
        &grown,
        &rules,
        "hostman/ci",
        Some(Balance {
            amount: 99.0,
            at_ms: now,
        }),
        ms("2026-12-24"),
    );
    assert!(
        refusals
            .iter()
            .any(|r| r.contains("the credit ended 2026-12-23")),
        "{refusals:?}"
    );
}

/// The balance as hostman-ci's job records it in its own database: the last reading and when.
#[dash_test]
fn the_balance_is_the_last_one_hostman_ci_recorded() {
    let mut log = MemoryLog::default();
    let write = |v: &str| FieldWrite {
        table: "ci_credit".into(),
        row: "hostman".into(),
        field: "balance".into(),
        value: dash_sdk::Value::Text(v.into()),
    };
    log.write(&[write("27.16")]).unwrap();
    log.write(&[write("26.90")]).unwrap();
    let read = balance(&log, 0).unwrap().expect("a reading");
    assert_eq!(read.amount, 26.90);
    assert!(balance(&MemoryLog::default(), 0).unwrap().is_none());
}

/// Point 6: tofu's stderr in an error, its source lines and its assignments left out: a value in the files never
/// reaches a caller's log; its stdout never at all.
#[dash_test]
fn an_error_says_tofus_diagnostic_without_values() {
    let stderr = "\nError: Unsupported argument\n\n  on main.tf line 3, in resource \"hm_server\" \"ci\":\n   3:   root_pass = \"hunter2\"\n\nAn argument named \"root_pass\" is not expected here.\n";
    let kept = said(stderr);
    assert!(
        kept.contains("Error: Unsupported argument") && kept.contains("on main.tf line 3"),
        "{kept}"
    );
    assert!(!kept.contains("hunter2"), "{kept}");
    // the values tofu prints under a snippet, framed or not (/code-review of the reviewed guard)
    let framed = "Error: Invalid function argument\n\n  on main.tf line 9:\n   9:   x = base64decode(local.ci)\n    ├────────────────\n    │ local.ci is \"#cloud-config token\"\n\nmore words\n";
    let kept = said(framed);
    assert!(
        !kept.contains("token") && kept.contains("more words"),
        "{kept}"
    );
}

/// A `moved` block takes a protected address elsewhere, whatever its action: refused, or the next PR deletes it
/// unprotected (/code-review of the reviewed guard).
#[dash_test]
fn a_protected_address_moved_away_is_refused() {
    let rules = Guard::parse(HOSTMAN_GUARD).unwrap();
    let now = ms("2026-11-18");
    for action in ["no-op", "update"] {
        let moved = Change {
            previous: Some("hm_server.ci".into()),
            ..change("hm_server.ci2", action, server(6273), server(6273))
        };
        let (refusals, _) = guard(&[moved, kept()[1].clone()], &rules, "hostman/ci", None, now);
        assert!(
            refusals
                .iter()
                .any(|r| r.contains("hm_server.ci is protected")),
            "{action}: {refusals:?}"
        );
    }
}

/// A host for the tests: the stand-in's files and clock, and `run` as the binary's (a cleared environment beside PATH
/// and HOME), so the plugin starts tofu the way it does live.
struct Here(StandIn);

impl WallClock for Here {
    fn now_ms(&self) -> i64 {
        self.0.now_ms
    }
}

impl Host for Here {
    fn repo(&self) -> PathBuf {
        self.0.repo.clone()
    }
    fn data(&self) -> PathBuf {
        self.0.data.clone()
    }
    fn home(&self) -> Result<PathBuf, String> {
        self.0.home()
    }
    fn arm(&self, _: &Path, _: &str) {}
    fn log(&self, _: &Path, _: &str, _: &[&str]) -> Result<Box<dyn OpLog>, String> {
        Ok(Box::new(MemoryLog::default()))
    }
    fn run(&self, p: &Program) -> Result<Ran, String> {
        let mut c = std::process::Command::new(p.bin);
        c.env_clear().args(p.args).envs(p.given.iter().copied());
        for kept in ["PATH", "HOME", "TMPDIR"] {
            if let Some(v) = std::env::var_os(kept) {
                c.env(kept, v);
            }
        }
        if let Some(dir) = p.dir {
            c.current_dir(dir);
        }
        let out = c.output().map_err(|e| format!("{}: {e}", p.bin))?;
        let text = |b: Vec<u8>| String::from_utf8_lossy(&b).into_owned();
        Ok(Ran {
            code: out.status.code().unwrap_or(-1),
            stdout: text(out.stdout),
            stderr: text(out.stderr),
        })
    }
}

/// Whether the real tofu answers here. In CI (`CI` set, as GitHub sets it) its absence fails the test: the rust
/// mirror installs tofu 1.13.1, so a skip there would hide the contract's tests (point 5).
fn have_tofu() -> bool {
    let ok = std::process::Command::new("tofu")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success());
    assert!(
        ok || std::env::var_os("CI").is_none(),
        "CI runs the tests against tofu: install tofu 1.13.1"
    );
    if !ok {
        eprintln!("tofu not found: the tests against it did not run");
    }
    ok
}

/// terraform_data needs no provider process (OpenTofu's built-in provider): the stacks below plan offline.
const DATA_GUARD: &str = r#"{
  "currency": "EUR",
  "expires": "2026-12-23",
  "types": {"terraform_data": {"monthly": 0}},
  "protected": {"data/one": ["terraform_data.ci"]}
}"#;

/// A checkout with infra/guard.json and the unit infra/data/<unit>: the host's repo is the test's folder.
fn checkout(name: &str, unit: &str, main_tf: &str) -> (TestDir, Here) {
    let dir = TestDir::new(name);
    std::fs::create_dir_all(dir.0.join("infra/data").join(unit)).unwrap();
    std::fs::write(dir.0.join("infra/guard.json"), DATA_GUARD).unwrap();
    std::fs::write(dir.0.join("infra/data").join(unit).join("main.tf"), main_tf).unwrap();
    let host = Here(StandIn {
        now_ms: ms("2026-11-18"),
        repo: dir.0.clone(),
        data: dir.0.join("data"),
    });
    (dir, host)
}

/// A state holding `terraform_data.<name>` with `input` "x", as tofu would have written it.
fn state_with(names: &[(&str, &[&str])]) -> String {
    let resources: Vec<Value> = names
        .iter()
        .map(|(name, deps)| {
            json!({"mode": "managed", "type": "terraform_data", "name": name,
                   "provider": "provider[\"terraform.io/builtin/terraform\"]",
                   "instances": [{"schema_version": 0, "dependencies": deps.iter().map(|d| format!("terraform_data.{d}")).collect::<Vec<_>>(),
                     "attributes": {"id": format!("id-{name}"), "input": {"value": "x", "type": "string"},
                                    "output": {"value": "x", "type": "string"}, "triggers_replace": null}}]})
        })
        .collect();
    json!({"version": 4, "terraform_version": "1.13.1", "serial": 1,
           "lineage": "6f0c3b1e-0000-4000-8000-000000000001", "outputs": {}, "resources": resources})
    .to_string()
}

fn run(host: &Here, args: &[&str]) -> (i32, Value) {
    let mut out = vec![];
    let code = cli(host, args, &mut out);
    let doc = if out.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&out).expect("one JSON document")
    };
    (code, doc)
}

/// No `.tfplan` left in the unit: the saved plan holds every value in clear.
fn no_saved_plan(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .all(|e| !e.file_name().to_string_lossy().ends_with(".tfplan"))
}

/// Point 1: a unit outside infra/, or one bringing its own guard.json, is refused; the central guard alone counts.
#[dash_test]
fn a_unit_is_under_infra_and_takes_the_central_guard_alone() {
    let (dir, host) = checkout("infra-where", "one", "output \"x\" {\n  value = 1\n}\n");
    let outside = TestDir::new("infra-outside");
    std::fs::write(
        outside.0.join("main.tf"),
        "output \"x\" {\n  value = 1\n}\n",
    )
    .unwrap();
    assert_eq!(
        run(&host, &["plan", &outside.0.display().to_string()]),
        (ERROR, Value::Null)
    );
    let unit = dir.0.join("infra/data/one");
    std::fs::write(unit.join("guard.json"), DATA_GUARD).unwrap();
    assert_eq!(
        run(&host, &["plan", &unit.display().to_string()]),
        (ERROR, Value::Null),
        "a unit's own guard"
    );
    std::fs::remove_file(dir.0.join("infra/guard.json")).unwrap();
    std::fs::remove_file(unit.join("guard.json")).unwrap();
    assert_eq!(
        run(&host, &["plan", &unit.display().to_string()]),
        (ERROR, Value::Null),
        "no central guard"
    );
}

/// `dash infra plan` against real tofu: a new unit plans two adds (exit 2); a unit matching its state plans nothing
/// (exit 0); a protected resource left out of the files plans a destroy the guard refuses (exit 3); a tofu error exits
/// 1 with nothing on stdout and leaves no saved plan.
#[dash_test]
fn plan_against_tofu_answers_the_contract_and_its_exit_codes() {
    if !have_tofu() {
        return;
    }
    let two = "resource \"terraform_data\" \"a\" {\n  input = \"x\"\n}\n\nresource \"terraform_data\" \"b\" {\n  input = terraform_data.a.id\n}\n";
    let (dir, host) = checkout("infra-fresh", "fresh", two);
    let unit = dir.0.join("infra/data/fresh");
    let (code, doc) = run(&host, &["plan", &unit.display().to_string()]);
    assert_eq!(code, CHANGES, "{doc}");
    assert_eq!(doc["contract"], CONTRACT);
    assert_eq!(doc["unit"], "data/fresh");
    assert_eq!(doc["counts"]["add"], 2, "{doc}");
    assert_eq!(doc["guard"]["passed"], true, "{doc}");
    assert!(
        !doc.to_string().contains("\"x\""),
        "no attribute value in the document: {doc}"
    );
    assert!(
        no_saved_plan(&unit) && no_saved_plan(&dir.0.join(crate::STATES)),
        "the saved plan, values in clear, is removed"
    );

    let one = "resource \"terraform_data\" \"ci\" {\n  input = \"x\"\n}\n";
    let (dir, host) = checkout("infra-same", "one", one);
    let unit = dir.0.join("infra/data/one");
    std::fs::write(unit.join("terraform.tfstate"), state_with(&[("ci", &[])])).unwrap();
    let (code, doc) = run(&host, &["plan", &unit.display().to_string()]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(
        doc["engine"]["version"], "1.13.1",
        "read from the plan tofu showed: {doc}"
    );
    // a unit named from the checkout, wherever dash runs (/code-review: the cwd's was read)
    let (code, doc) = run(&host, &["plan", "infra/data/one"]);
    assert_eq!(code, 0, "{doc}");

    std::fs::write(
        unit.join("main.tf"),
        "resource \"terraform_data\" \"other\" {\n  input = \"x\"\n}\n",
    )
    .unwrap();
    std::fs::write(
        unit.join("terraform.tfstate"),
        state_with(&[("ci", &[]), ("other", &[])]),
    )
    .unwrap();
    let (code, doc) = run(&host, &["plan", &unit.display().to_string()]);
    assert_eq!(code, REFUSED, "{doc}");
    assert!(
        doc["guard"]["refusals"]
            .to_string()
            .contains("terraform_data.ci is protected"),
        "{doc}"
    );

    std::fs::write(
        unit.join("main.tf"),
        "output \"x\" {\n  value = nosuchfn(1)\n}\n",
    )
    .unwrap();
    assert_eq!(
        run(&host, &["plan", &unit.display().to_string()]),
        (ERROR, Value::Null)
    );
    assert!(
        no_saved_plan(&unit) && no_saved_plan(&dir.0.join(crate::STATES)),
        "and leaves no saved plan"
    );
}

/// `dash infra state` and `graph` against real tofu's `show -json`: the allow-list holds (terraform_data's input and
/// output are not on it), and the state's dependencies become the graph's edges.
#[dash_test]
fn state_and_graph_against_tofu() {
    if !have_tofu() {
        return;
    }
    let two = "resource \"terraform_data\" \"a\" {\n  input = \"x\"\n}\n\nresource \"terraform_data\" \"b\" {\n  input = terraform_data.a.id\n}\n";
    let (dir, host) = checkout("infra-state", "two", two);
    let unit = dir.0.join("infra/data/two");
    std::fs::write(
        unit.join("terraform.tfstate"),
        state_with(&[("a", &[]), ("b", &["a"])]),
    )
    .unwrap();
    let path = unit.display().to_string();
    let (code, doc) = run(&host, &["state", &path]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["resources"][0]["address"], "terraform_data.a");
    assert_eq!(doc["resources"][0]["attributes"], json!({"id": "id-a"}));
    let (code, g) = run(&host, &["graph", &path]);
    assert_eq!(code, 0, "{g}");
    assert_eq!(
        g["edges"],
        json!([{"from": "terraform_data.b", "to": "terraform_data.a", "kind": "depends_on"}])
    );
    assert_eq!(
        run(&host, &["teleport", &path]),
        (ERROR, Value::Null),
        "an unknown word is an error"
    );
}

/// `state` and `graph` on `tofu show -json`'s state: attributes on the allow-list alone (a collection tofu marks {} or
/// [false] kept, one holding true not), child modules read, each depends_on an edge.
#[dash_test]
fn state_and_graph_keep_allowed_attributes_and_depends_on_edges() {
    let state = json!({"format_version": "1.0", "values": {"root_module": {
        "resources": [
            {"address": "hm_server.ci", "mode": "managed", "type": "hm_server", "name": "ci",
             "values": {"id": "9608429", "name": "ci", "os_id": 95, "cloud_init": "#cloud-config", "user_data": "x",
                        "root_pass": "hunter2", "status": "on", "tags": {"team": "a"}, "labels": {"k": "s"}},
             "sensitive_values": {"status": true, "tags": {}, "labels": {"k": true}}},
            {"address": "hm_floating_ip.ci", "mode": "managed", "type": "hm_floating_ip", "name": "ci",
             "values": {"id": "ba7d3127", "ip": "203.0.113.7"}, "sensitive_values": {}, "depends_on": ["hm_server.ci"]}
        ],
        "child_modules": [{"resources": [
            {"address": "module.dns.hm_domain.root", "mode": "managed", "type": "hm_domain", "name": "root",
             "values": {"id": "d1", "name": "whatsnr.eu"}, "sensitive_values": {}}
        ]}]
    }}});
    let found = resources(&state);
    assert_eq!(
        found[1]["attributes"],
        json!({"id": "9608429", "name": "ci", "os_id": 95, "tags": {"team": "a"}})
    );
    let g = graph(&state);
    assert_eq!(
        g["edges"],
        json!([{"from": "hm_floating_ip.ci", "to": "hm_server.ci", "kind": "depends_on"}])
    );
    assert_eq!(g["nodes"].as_array().unwrap().len(), 3);
}
