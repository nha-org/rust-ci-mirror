//! Task rust-audit-log through the store and the real binary: a plugin's tool, added by the plugin contract
//! (`Registry::add`, as drafts and mail add their slots), called over MCP, lands in the audit log with no line of its own;
//! `dash audit` sums it and lists the day's calls.
use dash_core::registry::{Contribution, Registry, Tool};
use dash_meter::TestDir;
use dash_meter::dash_test;
use std::process::Command;

const DASH: &str = env!("CARGO_BIN_EXE_dash");

/// A plugin's tool: it knows nothing of the log.
struct Servers;

impl Tool for Servers {
    fn description(&self) -> &str {
        "Cloud servers"
    }
    fn input_schema(&self) -> &str {
        r#"{"type":"object"}"#
    }
    fn call(&self, arguments: &str) -> Result<String, String> {
        match arguments.contains("create") {
            true => Err("402: past the spend cap".into()),
            false => Ok("2 servers".into()),
        }
    }
    fn cost(&self, _arguments: &str, answer: &Result<String, String>) -> f64 {
        if answer.is_ok() { 0.25 } else { 0.0 }
    }
}

fn dash(args: &[&str]) -> String {
    let output = Command::new(DASH).args(args).output().expect("dash");
    assert!(output.status.success(), "{args:?}: {output:?}");
    String::from_utf8(output.stdout).expect("utf-8")
}

#[dash_test]
fn a_plugin_tool_s_calls_land_in_the_audit_log_and_dash_audit_sums_them() {
    let dir = TestDir::new("audit-plugin");
    let fresh = TestDir::new("audit-fresh");
    let empty = dash(&[
        "audit",
        "hostman",
        "--data",
        fresh.0.to_str().expect("a path"),
    ]);
    assert!(
        empty.contains("hostman\t0\t0\t0\t0 ms\t"),
        "no store yet is no call yet: {empty}"
    );
    let mut registry = Registry::default();
    let plugin = Contribution {
        tools: vec![("hostman".into(), Box::new(Servers) as Box<dyn Tool>)],
        ..Default::default()
    };
    registry.add("hostman", plugin).expect("the plugin lands");
    let store = dash_store::open(&dir.0).expect("the store");
    let mcp = dash_surface::Mcp {
        name: "dash",
        instructions: "",
        registry: &registry,
        clock: &dash_store::oplog::SystemClock,
        audit: &store,
    };
    let call = |id: u8, args: &str| {
        format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"hostman","arguments":{args}}}}}"#
        )
    };
    let input = [
        call(1, "{}"),
        call(2, r#"{"list":1}"#),
        call(3, r#"{"create":"big"}"#),
    ]
    .join("\n");
    let mut output = vec![];
    dash_surface::serve_mcp(&mcp, input.as_bytes(), &mut output).expect("served");
    drop(store);

    let data = dir.0.to_str().expect("a path");
    let sum = dash(&["audit", "hostman", "--data", data]);
    assert!(
        sum.contains("hostman\t2\t1\t0.5\t") && sum.contains("\t402: past the spend cap"),
        "{sum}"
    );
    let none = dash(&["audit", "hostman", "--since", "2099-01-01", "--data", data]);
    assert!(none.contains("hostman\t0\t0\t0\t0 ms\t"), "{none}");
    let day = dash(&["audit", "--data", data]);
    assert!(day.contains("Each call"), "{day}");
    assert_eq!(day.matches("\thostman\tworked\t").count(), 2, "{day}");
    assert!(
        day.contains("\thostman\tfailed\t") && day.contains(r#"{"create":"big"}"#),
        "{day}"
    );
}
