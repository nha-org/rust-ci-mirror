use super::*;
use dash_core::registry::{Audit, Call, Registry};
use dash_meter::dash_test;
use serde_json::json;
use std::cell::RefCell;
use std::rc::Rc;

/// Hostman's answer to GET /api/v1/account/finances, recorded 2026-11-14 on his account (30 USD paid, nothing running).
const FINANCES_LIVE: &str = r#"{"finances":{"balance":30,"currency":"USD","discount_end_date_at":null,"discount_percent":0,"hourly_cost":0,"hourly_fee":0,"monthly_cost":0,"monthly_fee":0,"total_paid":30,"hours_left":null,"autopay_card_info":null,"total_balance":30},"response_id":"73086bef-7bdf-4738-af5a-12b184960b62"}"#;

/// Three of the 38 presets GET /api/v1/presets/servers answered on 2026-11-14, as recorded, and one with a GPU the
/// live list does not hold, as a configurator's `gpu` would name it.
const PRESETS: &str = r#"{"server_presets":[
{"id":5131,"location":"pl-1","price":35,"cpu":8,"cpu_frequency":"3","ram":16384,"disk":327680,"disk_type":"nvme","bandwidth":200,"description":"","description_short":"Hostman PL 320/2","is_allowed_local_network":true,"vds_node_configuration_name":"euro-kvmnvm","tags":["cp","discount_euro"],"is_dedicated_cpu":false},
{"id":5157,"location":"nl-1","price":20,"cpu":4,"cpu_frequency":"3.3","ram":8192,"disk":163840,"disk_type":"nvme","bandwidth":200,"description":"","description_short":"Hostman N 160/2","is_allowed_local_network":true,"vds_node_configuration_name":"nl_base","tags":["cp","nl_base"],"is_dedicated_cpu":false},
{"id":5161,"location":"nl-1","price":5,"cpu":2,"cpu_frequency":"3.3","ram":2048,"disk":61440,"disk_type":"nvme","bandwidth":200,"description":"","description_short":"Hostman N 60/2","is_allowed_local_network":true,"vds_node_configuration_name":"nl_base","tags":["cp","nl_base"],"is_dedicated_cpu":false},
{"id":9001,"location":"nl-1","price":3,"cpu":2,"ram":2048,"disk":61440,"gpu":1,"tags":["cp"]}],"meta":{"total":4}}"#;

/// One server running on preset 5161, at 5 USD a month, so the finances say 5 a month.
const SERVER: &str = r#"{"server":{"id":7,"name":"ci","preset_id":5161,"status":"on","root_pass":"hunter2","vnc_pass":"v","networks":[{"ips":[{"ip":"1.2.3.4"}]}]}}"#;
/// Two of the 17 OSes GET /api/v1/os/servers answered on 2026-11-14, as recorded, and a Windows the live list does not hold.
const OS: &str = r#"{"servers_os":[{"id":79,"family":"linux","name":"ubuntu","version":"22.04"},{"id":125,"family":"BSD","name":"freebsd","version":"13"},{"id":200,"family":"windows","name":"windows","version":"2022"}]}"#;
const FINANCES_RUNNING: &str = r#"{"finances":{"balance":30,"currency":"USD","hourly_cost":0.007,"monthly_cost":5,"total_paid":30}}"#;

/// A recorded API: each call is logged as « METHOD path », answered from `finances` and the lists above.
fn recorded(finances: &'static str) -> (Hostman, Rc<RefCell<Vec<String>>>) {
    let log = Rc::new(RefCell::new(vec![]));
    let kept = Rc::clone(&log);
    let http: Http = Box::new(move |method, path, _body| {
        kept.borrow_mut().push(format!("{method} {path}"));
        Ok(match (method, path) {
            ("GET", FINANCES) => finances,
            ("GET", "/api/v1/presets/servers") => PRESETS,
            ("GET", "/api/v1/os/servers") => OS,
            ("GET", "/api/v1/servers/7") => SERVER,
            ("GET", "/api/v1/servers/9") => r#"{"server":{"id":9,"preset_id":5157}}"#,
            ("GET", "/api/v1/servers/8") => {
                r#"{"server":{"id":8,"preset_id":null,"configurator_id":26}}"#
            }
            _ => r#"{"server":{"id":9}}"#,
        }
        .to_string())
    });
    (Hostman::new(http), log)
}

fn call(method: &str, path: &str, body: Value) -> String {
    json!({"method": method, "path": path, "body": body}).to_string()
}

/// Part 1: Cloud Servers alone; bare metal, GPU, AI and every other product are refused, and a refused call reaches no API.
#[dash_test]
fn only_cloud_servers_are_in_scope_bare_metal_gpu_ai_and_every_other_product_refused() {
    for (method, path) in [
        ("GET", "/api/v1/account/finances"),
        ("GET", "/api/v1/presets/servers"),
        ("GET", "/api/v1/configurator/servers"),
        ("GET", "/api/v1/os/servers"),
        ("GET", "/api/v1/servers"),
        ("GET", "/api/v1/servers?limit=10"),
        ("POST", "/api/v1/servers"),
        ("GET", "/api/v1/servers/7/statistics"),
        ("POST", "/api/v1/servers/7/reboot"),
        ("PATCH", "/api/v1/servers/7"),
        ("DELETE", "/api/v1/servers/7"),
    ] {
        assert_eq!(scope(method, path), Ok(()), "{method} {path}");
    }
    for (method, path) in [
        // bare metal
        ("GET", "/api/v1/dedicated-servers"),
        ("POST", "/api/v1/dedicated-servers"),
        // AI
        ("GET", "/api/v1/cloud-ai/agents"),
        ("POST", "/api/v1/cloud-ai/agents/1/call"),
        // every other product
        ("POST", "/api/v1/k8s/clusters"),
        ("POST", "/api/v1/databases"),
        ("POST", "/api/v1/storages/buckets"),
        ("POST", "/api/v1/balancers"),
        ("POST", "/api/v1/floating-ips"),
        ("POST", "/api/v2/apps"),
        ("PATCH", "/api/v1/account/finances"),
        ("POST", "/api/v1/presets/servers"),
        // a path that only looks like one
        ("GET", "/api/v1/serversx"),
        ("GET", "/api/v1/servers/../dedicated-servers"),
        ("GET", "/api/v1/servers/7%2F..%2F..%2Fdedicated-servers"),
        ("GET", "/api/v1/servers/abc"),
        ("DELETE", "/api/v1/servers"),
        ("PUT", "/api/v1/servers/7"),
        ("GET", "https://hostman.com/api/v1/servers"),
    ] {
        let why = scope(method, path).expect_err(path);
        assert!(
            why.contains("Cloud Servers (VPS) only"),
            "{method} {path}: {why}"
        );
    }
    let (tool, log) = recorded(FINANCES_LIVE);
    let gpu_config = json!({"name": "gpu", "configuration": {"configurator_id": 26, "cpu": 4, "ram": 8192, "disk": 81920, "gpu": 1}});
    let refusals = [
        tool.call(&call(
            "POST",
            "/api/v1/dedicated-servers",
            json!({"preset_id": 1}),
        )),
        tool.call(&call("POST", "/api/v1/cloud-ai/agents", json!({}))),
        // a GPU, asked through the configurator or a preset holding one
        tool.call(&call("POST", "/api/v1/servers", gpu_config)),
        tool.call(&call("POST", "/api/v1/servers", json!({"preset_id": 9001}))),
        // a write the tool cannot price
        tool.call(&call(
            "POST",
            "/api/v1/servers/7/disks",
            json!({"size": 10240}),
        )),
        tool.call(&call(
            "POST",
            "/api/v1/servers/7/ips",
            json!({"type": "ipv4"}),
        )),
        tool.call(&call(
            "PATCH",
            "/api/v1/servers/7",
            json!({"bandwidth": 1000}),
        )),
    ];
    for (i, refusal) in refusals.iter().enumerate() {
        assert!(
            refusal
                .as_ref()
                .is_err_and(|e| e.contains("refused") || e.contains("GPU")),
            "{i}: {refusal:?}"
        );
    }
    assert!(
        refusals[3]
            .as_ref()
            .unwrap_err()
            .contains("preset 9001 holds a GPU"),
        "{:?}",
        refusals[3]
    );
    let writes: Vec<String> = log
        .borrow()
        .iter()
        .filter(|c| !c.starts_with("GET"))
        .cloned()
        .collect();
    assert_eq!(
        writes,
        Vec::<String>::new(),
        "a refused call writes nothing"
    );
}

/// Part 2: the credit left and the month's spend, from the finances Hostman answered live.
#[dash_test]
fn usage_reads_the_credit_left_and_the_month_s_spend() {
    let (tool, log) = recorded(FINANCES_LIVE);
    assert_eq!(tool.usage(), Some(USAGE));
    assert_eq!(
        tool.call(USAGE),
        Ok("hostman: 30.00 USD of credit left; the month's spend 0.00 USD (0.00 an hour, the servers running now); 0.00 \
            USD spent of 30.00 paid"
            .into())
    );
    assert_eq!(*log.borrow(), ["GET /api/v1/account/finances"]);
    let running = usage_text(FINANCES_RUNNING).expect("read");
    assert!(
        running.contains("the month's spend 5.00 USD (0.01 an hour"),
        "{running}"
    );
    assert!(usage_text(r#"{"finances":{}}"#).is_err_and(|e| e.contains("no balance")));
}

/// Part 3: a create or resize whose month passes the credit left is refused with its cost, before any write; one
/// inside it goes through, and its cost is what it adds to the month.
#[dash_test]
fn a_create_or_resize_past_the_credit_left_is_refused_with_its_cost() {
    let (tool, log) = recorded(FINANCES_LIVE);
    let big = tool.call(&call(
        "POST",
        "/api/v1/servers",
        json!({"name": "big", "preset_id": 5131}),
    ));
    assert_eq!(
        big,
        Err("hostman: refused: a server on preset 5131 costs 35.00 USD a month, 35.00 USD a month with the servers \
             running, past the spend cap of 30.00 USD of credit left"
            .into())
    );
    assert_eq!(tool.cost("", &big), 0.0, "a refusal spends nothing");
    let small = call(
        "POST",
        "/api/v1/servers",
        json!({"name": "ci", "preset_id": 5161}),
    );
    let made = tool.call(&small);
    assert!(made.is_ok(), "{made:?}");
    assert_eq!(tool.cost(&small, &made), 5.0);
    let writes: Vec<String> = log
        .borrow()
        .iter()
        .filter(|c| !c.starts_with("GET"))
        .cloned()
        .collect();
    assert_eq!(
        writes,
        ["POST /api/v1/servers"],
        "the refused create never ran"
    );

    // server 7 runs at 5 a month: to 5131 adds 30, so 35 a month against 30 left; to 5157 adds 15, so 20
    let (tool, log) = recorded(FINANCES_RUNNING);
    let past = tool.call(&call(
        "PATCH",
        "/api/v1/servers/7",
        json!({"preset_id": 5131}),
    ));
    assert_eq!(
        past,
        Err("hostman: refused: server 7 resized to preset 5131 costs 30.00 USD a month, 35.00 USD a month with the \
             servers running, past the spend cap of 30.00 USD of credit left"
            .into())
    );
    let inside = call("PATCH", "/api/v1/servers/7", json!({"preset_id": 5157}));
    let resized = tool.call(&inside);
    assert!(resized.is_ok(), "{resized:?}");
    assert_eq!(tool.cost(&inside, &resized), 15.0);
    let clone = tool.call(&call("POST", "/api/v1/servers/7/clone", Value::Null));
    assert!(clone.is_err_and(|e| e.contains("it adds to the bill")));
    let unknown = tool.call(&call(
        "PATCH",
        "/api/v1/servers/8",
        json!({"preset_id": 5161}),
    ));
    assert!(unknown.is_err_and(|e| e.contains("server 8 has no preset")));
    let renamed = tool.call(&call("PATCH", "/api/v1/servers/7", json!({"name": "ci-2"})));
    assert!(renamed.is_ok(), "a rename costs nothing: {renamed:?}");
    let writes: Vec<String> = log
        .borrow()
        .iter()
        .filter(|c| !c.starts_with("GET"))
        .cloned()
        .collect();
    assert_eq!(
        writes,
        ["PATCH /api/v1/servers/7", "PATCH /api/v1/servers/7"],
        "the refused resizes never ran"
    );
}

#[derive(Default)]
struct Kept(RefCell<Vec<Call>>);

impl Audit for Kept {
    fn record(&self, call: &Call) -> Result<(), String> {
        self.0.borrow_mut().push(call.clone());
        Ok(())
    }
}

/// Part 4: registered as a plugin, its calls reach the core's audit log through `Registry::call`, with no line of its own.
#[dash_test]
fn its_calls_reach_the_audit_log_through_the_port_with_no_line_of_its_own() {
    let (tool, _) = recorded(FINANCES_LIVE);
    let mut registry = Registry::default();
    let plugin = Contribution {
        tools: vec![("hostman".into(), Box::new(tool) as Box<dyn Tool>)],
        ..Default::default()
    };
    registry.add("hostman", plugin).expect("it lands");
    let kept = Kept::default();
    let clock = || 1_000;
    let small = call("POST", "/api/v1/servers", json!({"preset_id": 5161}));
    for asked in [
        USAGE,
        &small,
        &call("POST", "/api/v1/dedicated-servers", json!({})),
    ] {
        let (_, recorded) = registry
            .call("hostman", asked, &clock, &kept)
            .expect("a tool");
        assert_eq!(recorded, Ok(()));
    }
    let calls = kept.0.into_inner();
    let kept: Vec<(&str, bool, f64)> = calls
        .iter()
        .map(|c| (c.tool.as_str(), c.answer.is_ok(), c.cost))
        .collect();
    assert_eq!(
        kept,
        [
            ("hostman", true, 0.0),
            ("hostman", true, 5.0),
            ("hostman", false, 0.0)
        ]
    );
    assert!(
        calls[2]
            .answer
            .as_ref()
            .is_err_and(|e| e.contains("Cloud Servers (VPS) only"))
    );
    let source = include_str!("lib.rs");
    for word in ["Audit", "record(", "eprintln", "println", "log::"] {
        assert!(!source.contains(word), "the tool's source holds {word}");
    }
}

/// The key is read from its file, by its variable; a file without it names the file, never a value.
#[dash_test]
fn the_key_comes_from_secrets_hostman_env() {
    let dir = dash_meter::TestDir::new("hostman-key");
    std::fs::create_dir_all(dir.0.join(".secrets")).expect("folder");
    let file = dir.0.join(KEY_FILE);
    assert!(key(&dir.0).is_err_and(|e| e.contains("hostman.env")));
    std::fs::write(&file, "OTHER=1\nHOSTMAN_API_KEY=\"abc\"\n").expect("write");
    assert_eq!(key(&dir.0), Ok("abc".into()));
    std::fs::write(&file, "HOSTMAN_API_KEY_OLD=s3cr3t-old\n").expect("write");
    assert!(
        key(&dir.0).is_err_and(|e| e.contains("holds no HOSTMAN_API_KEY") && !e.contains("s3cr3t"))
    );
}

/// /code-review's findings, each held: a glob curl would expand, an action that clones, a body key Hostman bills unpriced,
/// a downgrade's cost, the presets read once, and the key's usual env-file forms.
#[dash_test]
fn what_the_review_found_stays_refused_or_priced() {
    for path in [
        "/api/v1/servers?x=[1-20]",
        "/api/v1/servers/7/reboot?{a,b,c}",
        "/api/v1/servers/{7,8}",
    ] {
        assert!(scope("POST", path).is_err(), "{path}");
    }
    let (tool, log) = recorded(FINANCES_RUNNING);
    let clone = tool.call(&call(
        "POST",
        "/api/v1/servers/7/action",
        json!({"action": "clone"}),
    ));
    assert!(
        clone.is_err_and(|e| e.contains("a clone, which copies its disks")),
        "a clone copies what the tool does not price"
    );
    let reboot = tool.call(&call(
        "POST",
        "/api/v1/servers/7/action",
        json!({"action": "hard_reboot"}),
    ));
    assert!(
        reboot.is_ok() && tool.cost("", &reboot) == 0.0,
        "{reboot:?}"
    );
    let odd = tool.call(&call(
        "POST",
        "/api/v1/servers/7/action",
        json!({"action": "upgrade"}),
    ));
    assert!(odd.is_err_and(|e| e.contains("an action it does not know")));
    for (body, says) in [
        (
            json!({"preset_id": 5161, "is_ddos_guard": true}),
            "its is_ddos_guard",
        ),
        (
            json!({"preset_id": 5161, "software_id": 3}),
            "its software_id",
        ),
        (
            json!({"preset_id": 5161, "network": {"id": "n"}}),
            "its network",
        ),
        (
            json!({"preset_id": 5161, "bandwidth": 1000}),
            "more bandwidth than its own",
        ),
    ] {
        let made = tool.call(&call("POST", "/api/v1/servers", body.clone()));
        assert!(
            made.as_ref().is_err_and(|e| e.contains(says)),
            "{body}: {made:?}"
        );
    }
    let required =
        json!({"name": "ci", "preset_id": 5161, "is_ddos_guard": false, "bandwidth": 200});
    assert!(
        tool.call(&call("POST", "/api/v1/servers", required))
            .is_ok(),
        "what Hostman requires passes"
    );
    let patch = tool.call(&call(
        "PATCH",
        "/api/v1/servers/7",
        json!({"software_id": 3}),
    ));
    assert!(patch.is_err_and(|e| e.contains("its software_id")));

    // server 9 on 5157 (20) down to 5161 (5): the way down costs 0, never -15
    let (tool, log2) = recorded(FINANCES_RUNNING);
    let down = call("PATCH", "/api/v1/servers/9", json!({"preset_id": 5161}));
    let resized = tool.call(&down);
    assert!(
        resized.is_ok() && tool.cost(&down, &resized) == 0.0,
        "{resized:?}"
    );
    let presets = log2
        .borrow()
        .iter()
        .filter(|c| c.ends_with("/presets/servers"))
        .count();
    assert_eq!(
        presets,
        1,
        "a resize reads the presets once: {:?}",
        log2.borrow()
    );
    assert!(
        log.borrow().iter().all(|c| !c.contains('[')),
        "no glob reached the API"
    );

    let dir = dash_meter::TestDir::new("hostman-key-forms");
    std::fs::create_dir_all(dir.0.join(".secrets")).expect("folder");
    for line in [
        "export HOSTMAN_API_KEY='abc'",
        "HOSTMAN_API_KEY = abc",
        "  HOSTMAN_API_KEY=\"abc\"  ",
    ] {
        std::fs::write(dir.0.join(KEY_FILE), format!("{line}\n")).expect("write");
        assert_eq!(key(&dir.0), Ok("abc".into()), "{line}");
    }
}

/// The second /code-review's findings: a fragment, a deep path ending in a free verb, a body key on a clone or an action, and
/// a preset or bandwidth of the wrong type are each refused before any write; the key's inline comment is no part of it.
#[dash_test]
fn the_gate_lists_what_passes_never_what_does_not() {
    let (tool, log) = recorded(FINANCES_RUNNING);
    for (method, path, body, says) in [
        (
            "POST",
            "/api/v1/servers/7/clone#/start",
            Value::Null,
            "Cloud Servers (VPS) only",
        ),
        (
            "POST",
            "/api/v1/servers/7/disks/3/start",
            Value::Null,
            "it adds to the bill",
        ),
        (
            "PATCH",
            "/api/v1/servers/7",
            json!({"preset_id": "5131"}),
            "its preset_id",
        ),
        (
            "PATCH",
            "/api/v1/servers/7",
            json!({"preset_id": 5131.0}),
            "its preset_id",
        ),
        (
            "POST",
            "/api/v1/servers",
            json!({"preset_id": 5161, "is_ddos_guard": false, "bandwidth": "1000"}),
            "its bandwidth",
        ),
        (
            "POST",
            "/api/v1/servers/7/action",
            json!({"action": "install", "software_id": 3}),
            "it has no undo",
        ),
        (
            "POST",
            "/api/v1/servers/7/clone",
            json!({"preset_id": 5131}),
            "it adds to the bill",
        ),
        (
            "POST",
            "/api/v1/servers/7/reboot",
            json!({"bandwidth": 1000}),
            "its bandwidth",
        ),
        (
            "PUT",
            "/api/v1/servers/7",
            json!({}),
            "Cloud Servers (VPS) only",
        ),
    ] {
        let got = tool.call(&call(method, path, body.clone()));
        assert!(
            got.as_ref().is_err_and(|e| e.contains(says)),
            "{method} {path} {body}: {got:?}"
        );
    }
    let boot = tool.call(&call(
        "POST",
        "/api/v1/servers/7/boot-mode",
        json!({"boot_mode": "single"}),
    ));
    assert!(boot.is_ok(), "a free verb's own key passes: {boot:?}");
    let writes: Vec<String> = log
        .borrow()
        .iter()
        .filter(|c| !c.starts_with("GET"))
        .cloned()
        .collect();
    assert_eq!(
        writes,
        ["POST /api/v1/servers/7/boot-mode"],
        "nothing refused was written"
    );

    let dir = dash_meter::TestDir::new("hostman-key-comment");
    std::fs::create_dir_all(dir.0.join(".secrets")).expect("folder");
    for (line, key_is) in [
        ("HOSTMAN_API_KEY=abc # prod", "abc"),
        ("HOSTMAN_API_KEY=\"a#b\"", "a#b"),
    ] {
        std::fs::write(dir.0.join(KEY_FILE), format!("{line}\n")).expect("write");
        assert_eq!(key(&dir.0), Ok(key_is.into()), "{line}");
    }
}

/// The third /code-review's findings: no password reaches the audit log, a write with no undo is refused, an OS that may bill
/// a licence is refused, a write with no answer keeps its cost and says to look, and the cap's lock is held across the write.
#[dash_test]
fn no_password_kept_no_undo_no_licence_and_a_lost_answer_priced() {
    let (tool, log) = recorded(FINANCES_RUNNING);
    let read = tool
        .call(&call("GET", "/api/v1/servers/7", Value::Null))
        .expect("read");
    assert!(
        !read.contains("hunter2") && read.contains(r#""root_pass":"(masked)""#),
        "{read}"
    );
    assert!(
        read.contains("1.2.3.4") && !read.contains(r#""vnc_pass":"v""#),
        "{read}"
    );
    for (method, path, body) in [
        ("DELETE", "/api/v1/servers/7", Value::Null),
        ("DELETE", "/api/v1/servers/7/disks/3", Value::Null),
        (
            "POST",
            "/api/v1/servers/7/action",
            json!({"action": "remove"}),
        ),
        (
            "POST",
            "/api/v1/servers/7/action",
            json!({"action": "install"}),
        ),
    ] {
        let got = tool.call(&call(method, path, body.clone()));
        assert!(
            got.as_ref().is_err_and(|e| e.contains("it has no undo")),
            "{method} {path} {body}: {got:?}"
        );
    }
    let reinstall = tool.call(&call("PATCH", "/api/v1/servers/7", json!({"os_id": 79})));
    assert!(reinstall.is_err_and(|e| e.contains("its os_id")));
    for (os, passes) in [(79, true), (125, true), (200, false), (4, false)] {
        let made = tool.call(&call(
            "POST",
            "/api/v1/servers",
            json!({"preset_id": 5161, "os_id": os}),
        ));
        assert_eq!(made.is_ok(), passes, "os {os}: {made:?}");
    }
    let writes: Vec<String> = log
        .borrow()
        .iter()
        .filter(|c| !c.starts_with("GET"))
        .cloned()
        .collect();
    assert_eq!(
        writes,
        ["POST /api/v1/servers", "POST /api/v1/servers"],
        "nothing refused was written"
    );

    let dir = dash_meter::TestDir::new("hostman-lock");
    let at = dir.0.join("run/hostman.lock");
    let seen = at.clone();
    let held = Rc::new(Cell::new(None));
    let saw = Rc::clone(&held);
    let lost: Http = Box::new(move |method, path, _| match (method, path) {
        ("GET", FINANCES) => Ok(FINANCES_RUNNING.into()),
        ("GET", _) => Ok(PRESETS.into()),
        _ => {
            let other = File::open(&seen).expect("the lock file");
            saw.set(Some(matches!(
                other.try_lock(),
                Err(std::fs::TryLockError::WouldBlock)
            )));
            Err(format!("{NO_ANSWER}: curl: (28) Operation timed out"))
        }
    });
    let mut tool = Hostman::new(lost);
    tool.lock = Some(Box::new(move || at.clone()));
    let create = call("POST", "/api/v1/servers", json!({"preset_id": 5161}));
    let got = tool.call(&create);
    assert!(
        got.as_ref()
            .is_err_and(|e| e.contains("it may have gone through")),
        "{got:?}"
    );
    assert_eq!(
        tool.cost(&create, &got),
        5.0,
        "a write with no answer keeps its cost"
    );
    assert_eq!(held.get(), Some(true), "the lock is held across the write");
}

/// The fourth /code-review's findings: a create from an image is refused, masking keeps a private IP, a write curl may
/// have sent says so (a 5xx, a timeout) and one it never sent does not, and the lock file's month caps a second write
/// whose finances lag behind the first.
#[dash_test]
fn an_image_a_private_ip_a_lost_write_and_a_lagging_balance() {
    let (tool, log) = recorded(FINANCES_RUNNING);
    let image = tool.call(&call(
        "POST",
        "/api/v1/servers",
        json!({"preset_id": 5161, "image_id": "w"}),
    ));
    assert!(image.is_err_and(|e| e.contains("its image_id")));
    assert!(log.borrow().iter().all(|c| c.starts_with("GET")));

    let kept = masked(
        r#"{"is_private":true,"private_ip":"10.0.0.2","root_pass":"x","ssh_keys_ids":[1]}"#.into(),
    );
    assert_eq!(
        kept,
        r#"{"is_private":true,"private_ip":"10.0.0.2","root_pass":"(masked)","ssh_keys_ids":[1]}"#
    );

    for (exit, text, lost) in [
        (Some(0), "{}\n504", true),
        (Some(28), "", true),
        (Some(6), "", false),
        (Some(7), "", false),
        (Some(0), "{}\n422", false),
    ] {
        let got = answered(exit, text, "why");
        assert!(
            got.as_ref()
                .is_err_and(|e| e.starts_with(NO_ANSWER) == lost),
            "{exit:?} {text}: {got:?}"
        );
    }
    assert_eq!(
        answered(Some(0), "{\"a\":1}\n201", ""),
        Ok("{\"a\":1}".into())
    );

    let dir = dash_meter::TestDir::new("hostman-ledger");
    let at = dir.0.join("run/hostman.lock");
    let (mut tool, _) = recorded(FINANCES_RUNNING);
    tool.lock = Some(Box::new(move || at.clone()));
    let create = call("POST", "/api/v1/servers", json!({"preset_id": 5157}));
    assert!(
        tool.call(&create).is_ok(),
        "5 running and 20 more fit in 30"
    );
    let again = tool.call(&create);
    assert!(
        again.is_err_and(|e| e.contains("45.00 USD a month")),
        "the finances still say 5: the lock file's 25 stands in"
    );
}
