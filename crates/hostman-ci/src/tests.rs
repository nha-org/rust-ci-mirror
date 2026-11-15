use super::route::{Ci, TIERS, pick};
use super::*;
use dash_meter::dash_test;
use std::cell::RefCell;
use std::rc::Rc;

type Sent = Rc<RefCell<Vec<String>>>;

fn want() -> Want {
    Want {
        name: "ci-runner".into(),
        preset_id: 5157,
        ubuntu: "24.04".into(),
        bandwidth: 200,
        ssh_keys_ids: vec![1],
        repo: "nha-org/site_perso".into(),
    }
}

/// A recorded Hostman: `servers` is its list answer, `balance` and `monthly` the finances; each write is logged
/// as « METHOD path ».
fn hostman(servers: &'static str, balance: f64, monthly: f64) -> (Tool, Sent) {
    let sent: Sent = Rc::default();
    let kept = Rc::clone(&sent);
    let call = move |asked: &str| {
        let a: Json = serde_json::from_str(asked).unwrap();
        let (m, p) = (a["method"].as_str().unwrap(), a["path"].as_str().unwrap());
        if m != "GET" {
            kept.borrow_mut().push(format!("{m} {p}"));
        }
        Ok(match (m, p) {
            ("GET", "/api/v1/servers") => servers.to_string(),
            ("GET", "/api/v1/presets/servers") => {
                r#"{"server_presets":[{"id":5157,"price":20},{"id":5161,"price":5}]}"#.into()
            }
            ("GET", "/api/v1/account/finances") => {
                format!(r#"{{"finances":{{"balance":{balance},"monthly_cost":{monthly}}}}}"#)
            }
            ("GET", "/api/v1/os/servers") => {
                r#"{"servers_os":[{"id":79,"family":"linux","name":"ubuntu","version":"22.04"},{"id":99,"family":"linux","name":"ubuntu","version":"24.04"}]}"#.into()
            }
            ("POST", "/api/v1/servers") => {
                assert_eq!(a["body"]["os_id"], 99, "Ubuntu 24.04's id, from the list");
                r#"{"server":{"id":42}}"#.into()
            }
            _ => "{}".into(),
        })
    };
    (Box::new(call), sent)
}

fn runner(servers: &'static str, online: bool, balance: f64) -> (Runner, Sent) {
    let (call, sent) = hostman(servers, balance, 0.0);
    let r = Runner::new(
        want(),
        call,
        Box::new(move || Ok(online)),
        Box::new(|| Ok("TOKEN".into())),
    );
    (r, sent)
}

const ON: &str = r#"{"servers":[{"id":7,"name":"ci-runner","status":"on"}]}"#;
const OFF: &str = r#"{"servers":[{"id":7,"name":"ci-runner","status":"off"}]}"#;
const NONE: &str = r#"{"servers":[]}"#;

fn text(rows: &[FieldWrite], field: &str) -> Option<String> {
    rows.iter()
        .find(|r| r.field == field)
        .map(|r| match &r.value {
            Value::Text(t) => t.clone(),
            v => format!("{v:?}"),
        })
}

/// Part 1: one desired state: no server creates one, an off one starts, an up one is left, and
/// each result is rows through the write path.
#[dash_test]
fn one_server_with_a_runner_is_kept_up() {
    let (r, sent) = runner(NONE, false, 30.0);
    assert_eq!(text(&r.tick(), "status").unwrap(), "created server 42");
    assert_eq!(*sent.borrow(), ["POST /api/v1/servers"]);

    let (r, sent) = runner(OFF, false, 30.0);
    assert_eq!(text(&r.tick(), "status").unwrap(), "started server 7");
    assert_eq!(*sent.borrow(), ["POST /api/v1/servers/7/start"]);

    let (r, sent) = runner(ON, true, 30.0);
    assert_eq!(text(&r.tick(), "status").unwrap(), "up");
    assert!(sent.borrow().is_empty(), "an up server is left alone");
}

/// The create's body: preset, cloud-init with the label alone and two test threads, and no more than the tool lets through.
#[dash_test]
fn the_create_registers_the_runner_under_its_label_with_two_test_threads() {
    let init = cloud_init("nha-org/site_perso", "TOKEN");
    assert!(init.contains("--no-default-labels --labels site-perso-hostman"));
    assert!(init.contains("NEXTEST_TEST_THREADS=2"));
    assert!(init.contains("--url https://github.com/nha-org/site_perso --token TOKEN"));
    // what the gate's jobs load: the runner's own libraries, and the fontconfig main's dash links
    assert!(init.contains("bin/installdependencies.sh") && init.contains("libfontconfig1"));
}

/// Exactly one server (@nha, 2026-11-14: « take one 8GB machine please. »; /code-review of PR 1639): a list answer that
/// holds no list is no reading, so nothing is created on it, and an Ubuntu the list lacks creates nothing either.
#[dash_test]
fn a_list_that_does_not_read_creates_nothing() {
    for answer in ["{}", r#"{"message":"try later"}"#, r#"{"servers":null}"#] {
        let (r, sent) = runner(answer, false, 30.0);
        let alarm = text(&r.tick(), "alarm").unwrap();
        assert!(alarm.contains("holds no servers"), "{answer}: {alarm}");
        assert!(sent.borrow().is_empty(), "{answer}: nothing was created");
    }
    let (call, sent) = hostman(NONE, 30.0, 0.0);
    let old = Want {
        ubuntu: "12.04".into(),
        ..want()
    };
    let r = Runner::new(
        old,
        call,
        Box::new(|| Ok(false)),
        Box::new(|| Ok("T".into())),
    );
    assert!(
        text(&r.tick(), "alarm")
            .unwrap()
            .contains("no Ubuntu 12.04")
    );
    assert!(sent.borrow().is_empty());
}

/// Part 2: down is repaired (a wait, one reboot) or one alarm, never silence; the same reason is not sounded twice, and the
/// alarm clears when it is up again.
#[dash_test]
fn down_is_repaired_or_one_alarm_never_silence() {
    let (r, sent) = runner(ON, false, 30.0);
    for _ in 1..REBOOT_AT {
        let rows = r.tick();
        assert!(text(&rows, "status").unwrap().starts_with("waiting"));
        assert_eq!(text(&rows, "alarm"), None, "booting is not an alarm");
    }
    assert!(
        sent.borrow().is_empty(),
        "no reboot while cloud-init may still run"
    );
    assert!(text(&r.tick(), "status").unwrap().starts_with("rebooted"));
    assert_eq!(*sent.borrow(), ["POST /api/v1/servers/7/reboot"]);
    let mut alarm = None;
    for _ in REBOOT_AT..DOWN_AT {
        alarm = alarm.or(text(&r.tick(), "alarm"));
    }
    let alarm = alarm.expect("repair failed: an alarm");
    assert!(alarm.contains("no runner online after a reboot"), "{alarm}");
    let after = r.tick();
    assert!(
        text(&after, "status").unwrap().starts_with("down"),
        "the status stays true"
    );
    assert_eq!(text(&after, "alarm"), None, "one alarm for one reason");
    assert_eq!(sent.borrow().len(), 1, "one reboot only");

    // Hostman at work (installing) is waited on, never counted or rebooted; an unreadable GitHub never reboots
    let (r, sent) = runner(
        r#"{"servers":[{"id":7,"name":"ci-runner","status":"installing"}]}"#,
        false,
        30.0,
    );
    (0..30).for_each(|_| assert!(text(&r.tick(), "alarm").is_none()));
    assert!(sent.borrow().is_empty());
    let (call, sent) = hostman(ON, 30.0, 0.0);
    let blind = Runner::new(
        want(),
        call,
        Box::new(|| Err("HTTP 502".into())),
        Box::new(|| Ok(String::new())),
    );
    let alarms: Vec<_> = (0..DOWN_AT + 3)
        .filter_map(|_| text(&blind.tick(), "alarm"))
        .collect();
    assert_eq!(alarms.len(), 1, "{alarms:?}");
    assert!(alarms[0].contains("cannot be read: HTTP 502"));
    assert!(
        sent.borrow().is_empty(),
        "a server whose runner is unseen is never rebooted"
    );

    // a refused server (no money) is down at once, and the tool's own failure too
    let (r, _) = runner(
        r#"{"servers":[{"id":7,"name":"ci-runner","status":"no_paid"}]}"#,
        false,
        30.0,
    );
    assert!(text(&r.tick(), "alarm").unwrap().contains("no_paid"));
    let broken = Runner::new(
        want(),
        Box::new(|_| Err("hostman: no answer".into())),
        Box::new(|| Ok(true)),
        Box::new(|| Ok(String::new())),
    );
    assert_eq!(text(&broken.tick(), "alarm").unwrap(), "hostman: no answer");
}

/// Part 2's end: an alarm clears when the runner is up again.
#[dash_test]
fn the_alarm_clears_when_it_is_up() {
    let (call, _) = hostman(ON, 30.0, 0.0);
    let up = Rc::new(RefCell::new(false));
    let seen = Rc::clone(&up);
    let r = Runner::new(
        want(),
        call,
        Box::new(move || Ok(*seen.borrow())),
        Box::new(|| Ok(String::new())),
    );
    let alarm = (0..=DOWN_AT)
        .find_map(|_| text(&r.tick(), "alarm"))
        .unwrap();
    assert!(alarm.contains("no runner online"));
    *up.borrow_mut() = true;
    let rows = r.tick();
    assert_eq!(text(&rows, "status").unwrap(), "up");
    assert_eq!(text(&rows, "alarm").unwrap(), "", "cleared");
}

/// Part 3: the month's cost against the credit left, checked before each create: past it, no POST is sent and the alarm
/// names both numbers.
#[dash_test]
fn a_create_past_the_credit_left_is_refused_before_any_write() {
    let (r, sent) = runner(NONE, false, 15.0);
    let alarm = text(&r.tick(), "alarm").unwrap();
    assert!(
        alarm.contains("costs 20.00 a month") && alarm.contains("15.00 of credit left"),
        "{alarm}"
    );
    assert!(sent.borrow().is_empty(), "nothing was created");

    // the servers already running count: 20 + 10 running is past 25
    let (call, sent) = hostman(NONE, 25.0, 10.0);
    let r = Runner::new(
        want(),
        call,
        Box::new(|| Ok(false)),
        Box::new(|| Ok("T".into())),
    );
    assert!(
        text(&r.tick(), "alarm")
            .unwrap()
            .contains("30.00 with the servers running")
    );
    assert!(sent.borrow().is_empty());

    // and exactly the credit is allowed
    let (r, sent) = runner(NONE, false, 20.0);
    assert!(text(&r.tick(), "status").unwrap().starts_with("created"));
    assert_eq!(sent.borrow().len(), 1);
}

fn ci(id: &str, tier: &str, room: bool) -> Ci {
    Ci {
        id: id.into(),
        tier: tier.into(),
        room,
        labels: vec![id.into()],
    }
}

/// Part 4 (CiRouter): the Hostman runner is picked before the Mac, a free tier before it, and the whatsnr VPS's tier is gone.
#[dash_test]
fn ci_router_picks_the_hostman_runner_before_the_mac_and_the_vps_is_dropped() {
    let all = [
        ci("mac", "mac", true),
        ci("hostman", "hostman", true),
        ci("site-perso-vps", "vps", true),
    ];
    assert_eq!(pick(&all).unwrap().id, "hostman");
    let with_free = [
        ci("hostman", "hostman", true),
        ci("github", "monthly", true),
        ci("mac", "mac", true),
    ];
    assert_eq!(
        pick(&with_free).unwrap().id,
        "github",
        "free tiers keep their place"
    );
    let down = [
        ci("hostman", "hostman", false),
        ci("site-perso-vps", "vps", true),
        ci("mac", "mac", true),
    ];
    assert_eq!(
        pick(&down).unwrap().id,
        "mac",
        "the VPS is never picked, the Mac is the fallback"
    );
    assert!(!TIERS.contains(&"vps"));
    assert_eq!(TIERS[TIERS.len() - 2..], ["hostman", "mac"]);
    assert_eq!(pick(&[]), None);
}

/// His ask (@nha, 2026-11-14: « take one 8GB machine please. »): the one server is the cheapest 8 GB preset of the live list,
/// and its bandwidth is one that preset holds.
#[dash_test]
fn the_wanted_server_is_the_cheapest_8_gb_preset() {
    // the live list read 2026-11-15, the 8 GB ones: (id, price, bandwidth)
    let live = [
        (5133, 15, 100),
        (5145, 20, 200),
        (5157, 20, 200),
        (5515, 20, 200),
        (6273, 20, 200),
    ];
    let cheapest = live.iter().min_by_key(|p| p.1).unwrap();
    let want = Want::eight_gb("nha-org/site_perso");
    assert_eq!((want.preset_id, want.bandwidth), (cheapest.0, cheapest.2));
    assert_eq!(
        live.iter().filter(|p| p.1 == cheapest.1).count(),
        1,
        "no tie to break"
    );
    // one server: a listed one of that name is never created again
    let (r, sent) = runner(ON, true, 30.0);
    r.tick();
    assert!(sent.borrow().is_empty());
}

/// Done-when 3 of rust-hostman-ci-wire: `dash ci route` is route::pick over the older app's free pick, the runner and the
/// Mac. A free tier with room keeps the older app's JSON whole; its VPS pick goes to the runner when up, else the Mac.
#[dash_test]
fn dash_ci_route_picks_a_free_tier_then_the_hostman_runner_then_the_mac() {
    use super::route::{answer, asked};
    let words = |w: &str| -> Vec<String> { w.split(' ').map(String::from).collect() };
    assert_eq!(
        asked(&words("ci route --json --actions --branch b")),
        Some((true, "b".into()))
    );
    assert_eq!(asked(&words("ci route")), Some((false, "main".into())));
    assert_eq!(
        asked(&words("ci route circleci")),
        None,
        "another form goes to the older app"
    );
    assert_eq!(asked(&words("ci list")), None);
    let circle = r#"{"image":"ubuntu-24.04","provider":"circleci","runs-on":["circleci-linux-medium"],"why":"CircleCI for b: 133 of 3000 min used, monthly tier"}"#;
    let vps = r#"{"image":"ubuntu-24.04","provider":"vps","runs-on":["site-perso-vps"],"why":"the old whatsnr VPS for b: a slot free, vps tier"}"#;
    let json = |older: Result<&str, &str>, up: Result<bool, &str>| {
        let older = older.map(String::from).map_err(String::from);
        let (code, out, _) = answer(true, "b", older, up.map_err(String::from));
        assert_eq!(code, 0);
        serde_json::from_str::<Json>(&out).expect("one JSON line")
    };
    assert_eq!(
        json(Ok(circle), Ok(true))["provider"],
        "circleci",
        "a free tier with room first"
    );
    let hostman = json(Ok(vps), Ok(true));
    assert_eq!(
        hostman["runs-on"],
        json!(["site-perso-hostman"]),
        "the VPS's place is the runner's"
    );
    assert_eq!(hostman["provider"], "hostman");
    let mac = json(Ok(vps), Ok(false));
    assert_eq!(
        mac["runs-on"],
        json!(["self-hosted", "site-perso"]),
        "never the VPS: {mac}"
    );
    assert_eq!(
        json(Err("no answer"), Ok(true))["provider"],
        "hostman",
        "the older app down"
    );
    assert_eq!(json(Err("no answer"), Err("unread"))["provider"], "mac");
    let text = "vps: the old whatsnr VPS for main: a slot free, vps tier\n- CircleCI: full\n- the old whatsnr VPS: has room\n";
    let (_, said, _) = answer(false, "main", Ok(text.into()), Ok(true));
    assert_eq!(
        said,
        "hostman: the Hostman runner for main: up, hostman tier\n- CircleCI: full\n- the Hostman runner: up\n"
    );
}

/// Done-when 1 and 2 of rust-hostman-ci-wire: GitHub's list reads the runner online by its label; /status/ci-runner's lines
/// read the last status of the last minutes alone, say NOT OK with the alarm when Down, and say the job's silence.
#[dash_test]
fn the_status_page_shows_the_alarm_and_the_job_s_silence() {
    use super::live::{STALE_MS, lines, online, status};
    use dash_core::OpLog;
    let listed = |status: &str, label: &str| {
        format!(r#"{{"runners":[{{"status":"{status}","labels":[{{"name":"{label}"}}]}}]}}"#)
    };
    assert_eq!(online(&listed("online", LABEL)), Ok(true));
    assert_eq!(online(&listed("offline", LABEL)), Ok(false));
    assert_eq!(online(&listed("online", "site-perso-vps")), Ok(false));
    assert!(online("not json").is_err());
    let mut log = dash_core::MemoryLog::default();
    let (r, _) = runner(ON, true, 30.0);
    log.write(&r.tick()).unwrap();
    let at = log.0[0].stamp >> 16;
    assert_eq!(status(&log, at).unwrap(), Some(("up".into(), at)));
    assert_eq!(
        status(&log, at + STALE_MS + 1).unwrap(),
        None,
        "an old status is no reading"
    );
    let up = lines(Ok(Some(("up".into(), at))), at + 3_000);
    assert_eq!(up[0], "State: ok");
    assert_eq!(up[2], "Status: up (3 s ago)");
    assert_eq!(up[3], "Alarm: none");
    let down = lines(Ok(Some(("down: server 7 is blocked: his".into(), at))), at);
    assert_eq!(
        (down[0].as_str(), down[3].as_str()),
        ("State: NOT OK", "Alarm: server 7 is blocked: his")
    );
    let silent = lines(Ok(None), at);
    assert_eq!(silent[0], "State: NOT OK");
    assert!(silent[3].contains("the job is not running"), "{silent:?}");
}
