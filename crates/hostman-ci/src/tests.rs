use super::route::{Ci, TIERS, pick};
use super::*;
use dash_meter::dash_test;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

type Sent = Rc<RefCell<Vec<String>>>;

fn want() -> Want {
    Want {
        name: "ci-runner".into(),
        was: vec![],
        ram_mb: 8192,
        os: OS,
        ssh_keys_ids: vec![1],
        repo: "nha-org/site_perso".into(),
    }
}

/// Hostman's OS list as recorded 2026-11-15 (Debian 11 and 12, Ubuntu 22.04 and 24.04), with a Debian 13 at an id the live
/// list does not hold yet.
const OSES: &str = r#"{"servers_os":[{"id":67,"family":"linux","name":"debian","version":"11"},{"id":79,"family":"linux","name":"ubuntu","version":"22.04"},{"id":95,"family":"linux","name":"debian","version":"12"},{"id":99,"family":"linux","name":"ubuntu","version":"24.04"},{"id":141,"family":"linux","name":"debian","version":"13"}]}"#;

/// A recorded Hostman: `servers` is its list answer, `balance` and `monthly` the finances, `oses` its OS list; each write
/// is logged as « METHOD path body ».
fn hostman_with(
    servers: &'static str,
    oses: &'static str,
    balance: f64,
    monthly: f64,
) -> (Tool, Sent) {
    let sent: Sent = Rc::default();
    let kept = Rc::clone(&sent);
    let call = move |asked: &str| {
        let a: Json = serde_json::from_str(asked).unwrap();
        let (m, p) = (a["method"].as_str().unwrap(), a["path"].as_str().unwrap());
        if m != "GET" {
            let body = &a["body"];
            let shown = match body["cloud_init"].as_str() {
                // the cloud-init stands for its token alone, so a test reads which one went
                Some(init) => {
                    let token = init
                        .split("--token ")
                        .nth(1)
                        .and_then(|t| t.split(' ').next());
                    let mut b = body.clone();
                    b["cloud_init"] = json!(format!("token {}", token.unwrap_or("none")));
                    b.to_string()
                }
                None => body.to_string(),
            };
            kept.borrow_mut().push(format!("{m} {p} {shown}"));
        }
        Ok(match (m, p) {
            ("GET", "/api/v1/servers") => servers.to_string(),
            ("GET", "/api/v1/presets/servers") => {
                r#"{"server_presets":[{"id":5157,"location":"nl-1","price":20,"ram":8192,"bandwidth":200},{"id":5161,"location":"nl-1","price":5,"ram":2048,"bandwidth":200}]}"#.into()
            }
            ("GET", "/api/v1/account/finances") => {
                format!(r#"{{"finances":{{"balance":{balance},"monthly_cost":{monthly}}}}}"#)
            }
            ("GET", "/api/v1/os/servers") => oses.into(),
            ("POST", "/api/v1/servers") => {
                r#"{"server":{"id":42}}"#.into()
            }
            _ => "{}".into(),
        })
    };
    (Box::new(call), sent)
}

fn hostman(servers: &'static str, balance: f64, monthly: f64) -> (Tool, Sent) {
    hostman_with(servers, OSES, balance, monthly)
}

/// The writes sent, each as « METHOD path » alone.
fn paths(sent: &Sent) -> Vec<String> {
    let all = sent.borrow();
    all.iter()
        .map(|w| w.splitn(3, ' ').take(2).collect::<Vec<_>>().join(" "))
        .collect()
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

/// A runner on a clock the test moves, its tokens numbered T1, T2 in the order asked, GitHub answering `seen`.
fn clocked(
    call: Tool,
    want: Want,
    seen: Rc<RefCell<Result<bool, String>>>,
) -> (Runner, Rc<Cell<i64>>) {
    let clock = Rc::new(Cell::new(1_000_000));
    let (now, asked) = (Rc::clone(&clock), Rc::new(Cell::new(0)));
    let r = Runner::new(
        want,
        call,
        Box::new(move || seen.borrow().clone()),
        Box::new(move || {
            asked.set(asked.get() + 1);
            Ok(format!("T{}", asked.get()))
        }),
    )
    .clocked(Box::new(move || now.get()));
    (r, clock)
}

const MIN: i64 = 60_000;

const ON: &str = r#"{"servers":[{"id":7,"name":"ci-runner","status":"on","os":{"name":"debian","version":"13"}}]}"#;
const OFF: &str = r#"{"servers":[{"id":7,"name":"ci-runner","status":"off","os":{"name":"debian","version":"13"}}]}"#;
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
    assert_eq!(paths(&sent), ["POST /api/v1/servers"]);
    assert!(
        sent.borrow()[0].contains(r#""os_id":141"#),
        "the newest Debian listed, 13: {:?}",
        sent.borrow()
    );

    let (r, sent) = runner(OFF, false, 30.0);
    assert_eq!(text(&r.tick(), "status").unwrap(), "started server 7");
    assert_eq!(paths(&sent), ["POST /api/v1/servers/7/start"]);

    let (r, sent) = runner(ON, true, 30.0);
    assert_eq!(text(&r.tick(), "status").unwrap(), "up");
    assert!(sent.borrow().is_empty(), "an up server is left alone");
}

/// The create's body: the runner registered by name under the label alone, two test threads, GitHub's latest release (it
/// refuses to register one older than v2.329.0 since 2026-09-29), and what Debian 13 needs that its image may lack.
#[dash_test]
fn the_create_registers_the_runner_under_its_label_with_two_test_threads() {
    let init = cloud_init("nha-org/site_perso", "ci", "TOKEN");
    assert!(
        init.contains("--no-default-labels --labels ci --name ci"),
        "{init}"
    );
    assert!(init.contains("NEXTEST_TEST_THREADS=2"));
    assert!(init.contains("--url https://github.com/nha-org/site_perso --token TOKEN"));
    // what the gate's jobs load: the runner's own libraries, and the fontconfig main's dash links
    assert!(init.contains("bin/installdependencies.sh") && init.contains("libfontconfig1"));
    assert!(
        init.contains("releases/latest") && !init.contains("2.321"),
        "no pinned release"
    );
    assert!(
        init.contains("libicu76 || apt-get install -y libicu72") && init.contains(" sudo,"),
        "{init}"
    );
    assert!(!init.contains('~'), "no path that rests on sudo's HOME");
}

/// His OS (@nha, 2026-11-15: « debian. or alpine. », « don't pick ubuntu. »): the newest Debian listed. Hostman's list of
/// 2026-11-15 (Debian 11 and 12, Ubuntu 24.04) gives 12; a list with no Debian creates and wipes nothing; Ubuntu is refused
/// whatever the list holds.
#[dash_test]
fn the_os_is_the_newest_debian_listed_and_never_ubuntu() {
    assert_eq!(OS, "debian");
    assert_eq!(Want::eight_gb("o/r").os, OS);
    let live = r#"{"servers_os":[{"id":67,"family":"linux","name":"debian","version":"11"},{"id":99,"family":"linux","name":"ubuntu","version":"24.04"},{"id":95,"family":"linux","name":"debian","version":"12"}]}"#;
    let (call, sent) = hostman_with(NONE, live, 30.0, 0.0);
    let r = Runner::new(
        want(),
        call,
        Box::new(|| Ok(false)),
        Box::new(|| Ok("T".into())),
    );
    assert_eq!(text(&r.tick(), "status").unwrap(), "created server 42");
    assert!(
        sent.borrow()[0].contains(r#""os_id":95"#),
        "Debian 12: {:?}",
        sent.borrow()
    );

    let none = r#"{"servers_os":[{"id":99,"family":"linux","name":"ubuntu","version":"24.04"}]}"#;
    let (call, sent) = hostman_with(NONE, none, 30.0, 0.0);
    let r = Runner::new(
        want(),
        call,
        Box::new(|| Ok(false)),
        Box::new(|| Ok("T".into())),
    );
    let alarm = text(&r.tick(), "alarm").unwrap();
    assert!(alarm.contains("holds no debian"), "{alarm}");
    assert!(sent.borrow().is_empty());
    let ubuntu = Want {
        os: "ubuntu",
        ..want()
    };
    let (call, sent) = hostman(NONE, 30.0, 0.0);
    let r = Runner::new(
        ubuntu,
        call,
        Box::new(|| Ok(false)),
        Box::new(|| Ok("T".into())),
    );
    assert!(text(&r.tick(), "alarm").unwrap().contains("Ubuntu refused"));
    assert!(sent.borrow().is_empty());
}

/// Exactly one server (@nha, 2026-11-14: « take one 8GB machine please. »; /code-review of PR 1639): a list answer that
/// holds no list is no reading, so nothing is created on it, and an OS the list lacks creates nothing either.
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
        os: "alpine",
        ..want()
    };
    let r = Runner::new(
        old,
        call,
        Box::new(|| Ok(false)),
        Box::new(|| Ok("T".into())),
    );
    assert!(text(&r.tick(), "alarm").unwrap().contains("no alpine"));
    assert!(sent.borrow().is_empty());
}

/// Part 2: a server on with no runner online is waited on 15 minutes, then reinstalled with a fresh token (a PATCH of its
/// cloud-init, then « install »), at most once an hour; while it keeps failing, one alarm, never silence, and it clears when
/// the runner is up.
#[dash_test]
fn no_runner_after_15_minutes_is_one_reinstall_an_hour_and_one_alarm() {
    let (call, sent) = hostman(ON, 30.0, 0.0);
    let seen = Rc::new(RefCell::new(Ok(false)));
    let (r, clock) = clocked(call, want(), Rc::clone(&seen));
    let alarms = RefCell::new(vec![]);
    let tick = |r: &Runner| {
        let rows = r.tick();
        alarms.borrow_mut().extend(text(&rows, "alarm"));
        clock.set(clock.get() + MIN);
        text(&rows, "status").unwrap()
    };
    for _ in 0..15 {
        assert!(tick(&r).starts_with("waiting"));
    }
    assert!(
        sent.borrow().is_empty(),
        "no reinstall while cloud-init may still run"
    );
    assert_eq!(
        tick(&r),
        "reinstalled server 7 with a fresh registration token"
    );
    assert_eq!(
        *sent.borrow(),
        [
            r#"PATCH /api/v1/servers/7 {"cloud_init":"token T1"}"#,
            r#"POST /api/v1/servers/7/action {"action":"install"}"#
        ]
    );
    // 15 more minutes with no runner: one alarm, and no second reinstall within the hour
    let statuses: Vec<String> = (0..44).map(|_| tick(&r)).collect();
    assert!(
        statuses
            .last()
            .unwrap()
            .starts_with("down: server 7: no runner online after its reinstall")
    );
    assert_eq!(alarms.borrow().len(), 1, "{alarms:?}");
    assert_eq!(sent.borrow().len(), 2, "one reinstall within the hour");
    // an hour after the first, the second, with a fresh token; the alarm stands through it, never sounded again
    let second = (0..20)
        .map(|_| tick(&r))
        .find(|s| s.starts_with("reinstalled"));
    assert!(second.is_some(), "a reinstall an hour after the last");
    assert!(sent.borrow().iter().any(|w| w.contains("token T2")));
    (0..16).for_each(|_| {
        tick(&r);
    });
    assert_eq!(
        alarms.borrow().len(),
        1,
        "one alarm while it keeps failing: {alarms:?}"
    );
    *seen.borrow_mut() = Ok(true);
    assert_eq!(tick(&r), "up");
    assert_eq!(
        *alarms.borrow(),
        [
            "server 7: no runner online after its reinstall; the next is an hour after the last",
            ""
        ]
    );

    // Hostman at work (installing) is waited on, never counted or reinstalled
    let (r, sent) = runner(
        r#"{"servers":[{"id":7,"name":"ci-runner","status":"installing","os":{"name":"debian","version":"13"}}]}"#,
        false,
        30.0,
    );
    let r = r.clocked(Box::new(|| 0));
    (0..30).for_each(|_| assert!(text(&r.tick(), "alarm").is_none()));
    assert!(sent.borrow().is_empty());

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

/// Part 3 of the heal: a resolver or a GitHub that does not answer never creates or wipes a server. This Mac's DNS
/// (Tailscale's 100.100.100.100 left hostman.com unanswered six hours on 2026-11-15) is its own Down, said as such, one
/// alarm however long; an unreadable GitHub is waited on, then one alarm, never a reinstall.
#[dash_test]
fn a_dns_failure_or_an_unread_github_is_down_and_never_a_create_or_reinstall() {
    let dns = "hostman: not sent: this Mac's DNS does not resolve hostman.com (curl 6)";
    let writes = Rc::new(Cell::new(0));
    let w = Rc::clone(&writes);
    let call: Tool = Box::new(move |asked| {
        if !asked.contains(r#""method":"GET""#) {
            w.set(w.get() + 1);
        }
        Err(dns.into())
    });
    let (r, clock) = clocked(call, want(), Rc::new(RefCell::new(Ok(false))));
    let alarms: Vec<String> = (0..90)
        .filter_map(|_| {
            clock.set(clock.get() + MIN);
            text(&r.tick(), "alarm")
        })
        .collect();
    assert_eq!(alarms, [dns], "one alarm that says DNS");
    assert_eq!(writes.get(), 0, "nothing created or reinstalled");

    let (call, sent) = hostman(ON, 30.0, 0.0);
    let (blind, clock) = clocked(call, want(), Rc::new(RefCell::new(Err("HTTP 502".into()))));
    let alarms: Vec<String> = (0..90)
        .filter_map(|_| {
            clock.set(clock.get() + MIN);
            text(&blind.tick(), "alarm")
        })
        .collect();
    assert_eq!(alarms.len(), 1, "{alarms:?}");
    assert!(alarms[0].contains("cannot be read: HTTP 502"));
    assert!(
        sent.borrow().is_empty(),
        "a server whose runner is unseen is never wiped"
    );
}

/// What server 9602127 was on 2026-11-15: on, no IP in its public network (Hostman's GET .../ips answered none), so its
/// cloud-init reached no apt and no GitHub. An IPv4 is added, then the next tick reinstalls it at once.
#[dash_test]
fn a_server_with_no_public_ip_is_given_one_then_reinstalled() {
    let bare = r#"{"servers":[{"id":7,"name":"ci-runner","status":"on","os":{"name":"debian","version":"13"},"networks":[{"bandwidth":200,"blocked_ports":[],"ips":[],"type":"public"}]}]}"#;
    let (call, sent) = hostman(bare, 30.0, 0.0);
    let (r, _) = clocked(call, want(), Rc::new(RefCell::new(Ok(false))));
    assert!(
        text(&r.tick(), "status")
            .unwrap()
            .starts_with("addressed server 7")
    );
    assert_eq!(
        *sent.borrow(),
        [r#"POST /api/v1/servers/7/ips {"type":"ipv4"}"#]
    );
    // a list that names no networks is no reading: nothing is bought on it
    let (r, sent) = runner(ON, false, 30.0);
    r.tick();
    assert!(sent.borrow().is_empty());
    // once addressed, the next tick reinstalls without the 15 minutes' wait
    let (call, sent) = hostman(ON, 30.0, 0.0);
    let r = Runner::new(
        want(),
        call,
        Box::new(|| Ok(false)),
        Box::new(|| Ok("T".into())),
    )
    .seeded(State {
        since: Some(-HEAL_MS),
        ..State::default()
    });
    assert!(
        text(&r.tick(), "status")
            .unwrap()
            .starts_with("reinstalled")
    );
    assert_eq!(
        paths(&sent),
        ["PATCH /api/v1/servers/7", "POST /api/v1/servers/7/action"]
    );
}

/// The coordinator's three of 2026-11-15 (@nha: « site-perso-ci => just ci. »): server 9602127, site-perso-ci on Ubuntu, is
/// renamed ci and reinstalled on Debian 13 in one PATCH (a new os_id reinstalls it, so no « install » follows), once, with
/// a fresh token, and never a create beside it.
#[dash_test]
fn the_old_ubuntu_box_is_renamed_and_reinstalled_on_debian_never_a_second() {
    let old = r#"{"servers":[{"id":9602127,"name":"site-perso-ci","status":"on","os":{"id":99,"name":"ubuntu","version":"24.04"},"networks":[{"ips":[{"ip":"5.6.7.8","type":"ipv4"}],"type":"public"}]}]}"#;
    let (call, sent) = hostman(old, 30.0, 20.0);
    let (r, clock) = clocked(
        call,
        Want::eight_gb("nha-org/site_perso"),
        Rc::new(RefCell::new(Ok(false))),
    );
    assert_eq!(
        text(&r.tick(), "status").unwrap(),
        "reinstalled server 9602127 with a fresh registration token"
    );
    assert_eq!(
        *sent.borrow(),
        [r#"PATCH /api/v1/servers/9602127 {"cloud_init":"token T1","name":"ci","os_id":141}"#]
    );
    // the list still says site-perso-ci on Ubuntu (Hostman installing lags): no second reinstall within the hour, no create
    for _ in 0..30 {
        clock.set(clock.get() + MIN);
        r.tick();
    }
    assert_eq!(sent.borrow().len(), 1, "{:?}", sent.borrow());
    assert!(
        sent.borrow()
            .iter()
            .all(|w| !w.starts_with("POST /api/v1/servers ")),
        "zero creates"
    );
}

/// A restart resumes the state: since when no runner was online and the last reinstall, from the job's own log, so a
/// deploy every ten minutes never holds the reinstall off, and never lets a second in within the hour.
#[dash_test]
fn a_restart_resumes_the_wait_and_the_last_reinstall_from_the_log() {
    use super::live::seed;
    // the statuses a minute apart, at the stamps the core's log gives (ms << 16)
    let at = |i: i64| 10_000_000 + i * MIN;
    let statuses = [
        "up".to_string(),
        Step::Reinstalled(7).text(),
        "waiting: server 7 is installing".into(),
        Step::Waiting(7).text(),
        Step::Waiting(7).text(),
    ];
    let ops = (statuses.iter().zip(0..)).map(|(t, i)| dash_core::Op {
        stamp: at(i) << 16,
        writer: "t".into(),
        write: FieldWrite {
            table: "ci_runner".into(),
            row: "ci".into(),
            field: "status".into(),
            value: Value::Text(t.clone()),
        },
    });
    let log = dash_core::MemoryLog(ops.collect(), false);
    let state = seed(&log, at(4)).unwrap();
    assert_eq!(state.installed, Some(at(1)), "{state:?}");
    assert_eq!(
        state.since,
        Some(at(3)),
        "the run of no runner began after the install: {state:?}"
    );
    let later = seed(&log, at(4) + REINSTALL_MS + MIN).unwrap();
    assert_eq!(later, State::default(), "past the hour, nothing kept");
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

/// The 8 GB presets of the live list, read 2026-11-15 and given out of order, beside a 2 GB one and one with a GPU.
const LIVE: &str = r#"{"server_presets":[
{"id":6273,"location":"de-1","price":20,"ram":8192,"bandwidth":200},
{"id":5157,"location":"nl-1","price":20,"ram":8192,"bandwidth":200},
{"id":5133,"location":"pl-1","price":15,"ram":8192,"bandwidth":100},
{"id":5515,"location":"us-3","price":20,"ram":8192,"bandwidth":200},
{"id":5145,"location":"us-2","price":20,"ram":8192,"bandwidth":200},
{"id":5161,"location":"nl-1","price":5,"ram":2048,"bandwidth":200},
{"id":9001,"location":"nl-1","price":3,"ram":8192,"gpu":1}]}"#;

/// His ask (@nha, 2026-11-14: « take one 8GB machine please. »): the one server holds 8 GB, taken cheapest first from the
/// live list, ties by id, never a GPU's.
#[dash_test]
fn the_wanted_server_is_the_cheapest_8_gb_preset() {
    let want = Want::eight_gb("nha-org/site_perso");
    assert_eq!(want.ram_mb, 8192);
    let live: Json = serde_json::from_str(LIVE).unwrap();
    let order: Vec<i64> = (candidates(&live, want.ram_mb, &[]).iter())
        .map(|p| p["id"].as_i64().unwrap())
        .collect();
    assert_eq!(order, [5133, 5145, 5157, 5515, 6273]);
    // one server: a listed one of that name is never created again
    let (r, sent) = runner(ON, true, 30.0);
    r.tick();
    assert!(sent.borrow().is_empty());
}

/// The first create's answer, 2026-11-15 (task hostman-ci-no-free-node): Hostman made nothing in pl-1.
const NO_NODE: &str =
    r#"hostman answered 409: {"error_code":"no_free_node","message":"No free node"}"#;

/// A Hostman with the live 8 GB presets and no server, whose creates answer `posts` in turn; each create sent is logged as
/// « <preset> <bandwidth> ».
fn crowded(posts: Vec<Result<&'static str, &'static str>>) -> (Tool, Sent) {
    let sent: Sent = Rc::default();
    let (kept, posts) = (Rc::clone(&sent), RefCell::new(posts.into_iter()));
    let call = move |asked: &str| {
        let a: Json = serde_json::from_str(asked).unwrap();
        match (a["method"].as_str().unwrap(), a["path"].as_str().unwrap()) {
            ("GET", "/api/v1/servers") => Ok(NONE.into()),
            ("GET", "/api/v1/presets/servers") => Ok(LIVE.into()),
            ("GET", "/api/v1/account/finances") => {
                Ok(r#"{"finances":{"balance":30,"monthly_cost":0}}"#.into())
            }
            ("GET", "/api/v1/os/servers") => Ok(OSES.into()),
            ("POST", "/api/v1/servers") => {
                let b = &a["body"];
                kept.borrow_mut()
                    .push(format!("{} {}", b["preset_id"], b["bandwidth"]));
                let answer = posts
                    .borrow_mut()
                    .next()
                    .expect("no create past the ones planned");
                answer.map(String::from).map_err(String::from)
            }
            (m, p) => panic!("{m} {p} was not asked for"),
        }
    };
    (Box::new(call), sent)
}

/// Task hostman-ci-no-free-node (the first tick, 2026-11-15: « hostman answered 409: no_free_node »): a create refused for
/// want of a free node is remembered with its location, the next tick creates on the next 8 GB preset elsewhere, one create a
/// tick; once every one refused, one stable alarm, and an hour later the first is asked again.
#[dash_test]
fn a_location_with_no_free_node_gives_way_to_the_next_8_gb_preset() {
    let at = Rc::new(std::cell::Cell::new(1_000_i64));
    let clock = Rc::clone(&at);
    let runner = |posts| {
        let (call, sent) = crowded(posts);
        let clock = Rc::clone(&clock);
        let r = Runner::new(
            want(),
            call,
            Box::new(|| Ok(false)),
            Box::new(|| Ok("T".into())),
        );
        (r.clocked(Box::new(move || clock.get())), sent)
    };
    let (r, sent) = runner(vec![Err(NO_NODE), Ok(r#"{"server":{"id":42}}"#)]);
    let first = r.tick();
    assert_eq!(
        text(&first, "status").unwrap(),
        "no free node: preset 5133 in pl-1 refused; the next tick tries 5145 in us-2"
    );
    assert_eq!(
        text(&first, "alarm"),
        None,
        "a refusal with a next preset is no alarm"
    );
    assert_eq!(*sent.borrow(), ["5133 100"], "one create a tick");
    at.set(61_000);
    assert_eq!(text(&r.tick(), "status").unwrap(), "created server 42");
    assert_eq!(
        *sent.borrow(),
        ["5133 100", "5145 200"],
        "the next one, with its own bandwidth"
    );

    let (r, sent) = runner(vec![Err(NO_NODE); 6]);
    at.set(1_000);
    for _ in 0..5 {
        r.tick();
    }
    assert_eq!(
        *sent.borrow(),
        ["5133 100", "5145 200", "5157 200", "5515 200", "6273 200"]
    );
    let rows = r.tick();
    let alarm = text(&rows, "alarm").unwrap();
    assert!(alarm.starts_with("no free node in any preset"), "{alarm}");
    assert_eq!(sent.borrow().len(), 5, "none asked again within the hour");
    assert_eq!(
        text(&r.tick(), "alarm"),
        None,
        "the same reason sounds once"
    );
    at.set(1_000 + NO_NODE_MS);
    r.tick();
    assert_eq!(
        sent.borrow().last().unwrap(),
        "5133 100",
        "an hour on, the cheapest is asked again"
    );
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
        json!(["ci"]),
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
    let listed = |status: &str, name: &str, label: &str| {
        format!(
            r#"{{"runners":[{{"name":"{name}","status":"{status}","labels":[{{"name":"{label}"}}]}}]}}"#
        )
    };
    assert_eq!(online(&listed("online", "ci", LABEL), "ci"), Ok(true));
    assert_eq!(online(&listed("offline", "ci", LABEL), "ci"), Ok(false));
    assert_eq!(
        online(&listed("online", "ci", "site-perso-vps"), "ci"),
        Ok(false)
    );
    assert!(online("not json", "ci").is_err());
    // what GitHub listed on 2026-11-15: the whatsnr VPS runners carry the label as a bridge, and are not this box's runner
    let bridge = r#"{"runners":[{"name":"whatsnr-prod-1-site-perso-vps","status":"online","labels":[{"name":"site-perso-vps"},{"name":"site-perso-hostman"},{"name":"ci"}]}]}"#;
    assert_eq!(online(bridge, "ci"), Ok(false));
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
