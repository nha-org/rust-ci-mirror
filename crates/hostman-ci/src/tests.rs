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
        mbps: 0,
        os: OS,
        ssh_keys_ids: vec![1],
        jobs: 3,
        redirect: true,
        registrations: vec!["nha-org/site_perso".into()],
        leave_until_ms: None,
    }
}

/// The 4 GB box ci-2 by his leave (@nha, 2026-11-18: « feel free to lift gates if this is needed or spin up temporary
/// hostman servers. »): with the 8 GB box down and its credit spent, ci-2 is created past the credit left until the
/// credit's expiry, 2026-12-23, its status saying so; from that day, and for any box without the leave, the cap holds.
#[dash_test]
fn ci_2_is_created_past_the_credit_by_his_leave_until_the_23rd() {
    assert_eq!(Want::four_gb(&["r"]).leave_until_ms, Some(LEAVE_UNTIL_MS));
    assert_eq!(
        Want::eight_gb(&["r"]).leave_until_ms,
        None,
        "every other create keeps the cap"
    );
    assert_eq!(
        dash_core::iso(LEAVE_UNTIL_MS / 1000),
        "2026-12-23T00:00:00Z"
    );
    let on = |ms: i64| {
        let (call, sent) = hostman(NONE, 15.0, 0.0);
        let want = Want {
            leave_until_ms: Some(LEAVE_UNTIL_MS),
            ..want()
        };
        let r = Runner::new(
            want,
            call,
            Box::new(|_| Ok(Seen::Gone)),
            Box::new(|_| Ok("T".into())),
        );
        (r.clocked(Box::new(move || ms)), sent)
    };
    // 20 a month against 15 left: past the credit, by his leave
    let (r, sent) = on(LEAVE_UNTIL_MS - 1);
    assert_eq!(
        text(&r.tick(), "status").unwrap(),
        "created server 42, past the credit cap by his leave until 2026-12-23"
    );
    assert_eq!(paths(&sent), ["POST /api/v1/servers"]);
    // the day the credit expires, the cap again
    let (r, sent) = on(LEAVE_UNTIL_MS);
    let alarm = text(&r.tick(), "alarm").unwrap();
    assert!(alarm.contains("past the 15 of credit left"), "{alarm}");
    assert!(sent.borrow().is_empty());
    // the leave lets the credit's refusal alone through: a finances read that fails, or holds no balance, creates nothing
    for finances in [
        Err("hostman: no answer".to_string()),
        Ok(r#"{"finances":{}}"#.to_string()),
    ] {
        let (inner, sent) = hostman(NONE, 15.0, 0.0);
        let call: Tool =
            Box::new(
                move |asked: &str| match asked.contains("/api/v1/account/finances") {
                    true => finances.clone(),
                    false => inner(asked),
                },
            );
        let want = Want {
            leave_until_ms: Some(LEAVE_UNTIL_MS),
            ..want()
        };
        let r = Runner::new(
            want,
            call,
            Box::new(|_| Ok(Seen::Gone)),
            Box::new(|_| Ok("T".into())),
        );
        let rows = r.clocked(Box::new(|| 0)).tick();
        assert!(
            text(&rows, "status").unwrap().starts_with("down: "),
            "{:?}",
            text(&rows, "status")
        );
        assert!(
            sent.borrow().is_empty(),
            "nothing created on an unread credit"
        );
    }
    // inside the credit, a plain create: the leave is said only where it was used
    let (call, _) = hostman(NONE, 30.0, 0.0);
    let want = Want {
        leave_until_ms: Some(LEAVE_UNTIL_MS),
        ..want()
    };
    let r = Runner::new(
        want,
        call,
        Box::new(|_| Ok(Seen::Gone)),
        Box::new(|_| Ok("T".into())),
    );
    assert_eq!(
        text(&r.clocked(Box::new(|| 0)).tick(), "status").unwrap(),
        "created server 42"
    );
}

/// The 8 GB box's cloud-init for `tokens`, each a runner under `LABEL`, root taking `keys`.
fn init(tokens: &[(&str, String)], keys: &[String]) -> String {
    let regs: Vec<Reg> = tokens.iter().map(|(r, _)| Reg::from(*r)).collect();
    let tokens: Vec<(&Reg, String)> = (regs.iter().zip(tokens))
        .map(|(r, (_, t))| (r, t.clone()))
        .collect();
    cloud_init(&Want::eight_gb(&[]), &tokens, keys)
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
                // the cloud-init stands for its tokens alone, one per registration, so a test reads which went
                Some(init) => {
                    let tokens: Vec<&str> = (init.split("--token ").skip(1))
                        .filter_map(|t| t.split(' ').next())
                        .collect();
                    let mut b = body.clone();
                    let shown = if tokens.is_empty() {
                        "none".into()
                    } else {
                        tokens.join(" ")
                    };
                    b["cloud_init"] = json!(format!("token {shown}"));
                    // the cloud-init's hash stands as « sha », its one value checked by its own test
                    if b["comment"]
                        .as_str()
                        .is_some_and(|c| c.starts_with("cloud-init sha256 "))
                    {
                        b["comment"] = json!("sha");
                    }
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
            ("GET", p) if p.starts_with("/api/v1/servers/7/statistics?date_from=") => STATS.into(),
            ("POST", "/api/v1/servers") => {
                r#"{"server":{"id":42}}"#.into()
            }
            // Hostman's 204s answer nothing (its OpenAPI: start and each action), as /action did at 12:02 on 2026-11-15
            ("POST", p) if p.ends_with("/start") || p.ends_with("/action") => String::new(),
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
        Box::new(move |_| Ok(online.into())),
        Box::new(|_| Ok("TOKEN".into())),
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
        Box::new(move |_| seen.borrow().clone().map(Seen::from)),
        Box::new(move |_| {
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
    let init = init(&[("nha-org/site_perso", "TOKEN".into())], &[]);
    assert!(
        init.contains("--no-default-labels --labels ci,ci-light --name ci"),
        "{init}"
    );
    assert!(init.contains("NEXTEST_TEST_THREADS=2"));
    assert!(init.contains("--url https://github.com/nha-org/site_perso --token TOKEN"));
    // what the gate's jobs load: the runner's own libraries, and the fontconfig main's dash links
    assert!(init.contains("bin/installdependencies.sh") && init.contains("libfontconfig1"));
    // the resolver before apt: 9602127 waited one 5 s resolver timeout per connection (gate run 43710566004)
    assert!(
        init.find(RESOLVER).unwrap() < init.find("package_update").unwrap()
            && init.contains("bootcmd:\n  - rm -f /etc/resolv.conf && printf 'nameserver 1.1.1.1\\nnameserver 8.8.8.8\\noptions timeout:1 attempts:2 single-request-reopen\\n' > /etc/resolv.conf\n  \
                - grep -qs supersede /etc/dhcp/dhclient.conf || echo 'supersede domain-name-servers 1.1.1.1, 8.8.8.8;' >> /etc/dhcp/dhclient.conf || true\n"),
        "{init}"
    );
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

/// Done-when 1 of hostman-ci-klar-runner: `Want` holds two registrations, this repository and the klar-im organisation, and
/// the one cloud-init gives each its own runner process, folder and service, both named and labelled `ci`, each with its
/// own token and URL, the shared libraries installed before either registers.
#[dash_test]
fn want_holds_two_registrations_each_its_own_runner_and_service_labelled_ci() {
    use super::live::{REGISTRATIONS, api};
    let want = Want::eight_gb(&REGISTRATIONS);
    assert_eq!(
        want.registrations,
        [Reg::from("nha-org/site_perso"), Reg::from("klar-im")]
    );
    assert_eq!(api("nha-org/site_perso"), "repos/nha-org/site_perso");
    assert_eq!(
        api("klar-im"),
        "orgs/klar-im",
        "an organisation's runners and tokens"
    );
    let init = init(
        &[
            ("nha-org/site_perso", "TA".into()),
            ("klar-im", "TB".into()),
        ],
        &[],
    );
    for (dir, url) in [
        (
            "/home/runner/nha-org-site_perso-ci",
            "https://github.com/nha-org/site_perso --token TA",
        ),
        (
            "/home/runner/klar-im-ci",
            "https://github.com/klar-im --token TB",
        ),
    ] {
        assert!(
            init.contains(&format!("tar -xzf /home/runner/r.tgz -C {dir}\n")),
            "{init}"
        );
        let config = format!("cd {dir} && echo NEXTEST_TEST_THREADS=2");
        assert!(init.contains(&config) && init.contains(url), "{init}");
        assert!(
            init.contains(&format!(
                "cd {dir} && ./svc.sh install runner && ./svc.sh start"
            )),
            "{init}"
        );
    }
    assert_eq!(
        init.matches("--labels ci,ci-light --name ci --url").count(),
        2,
        "both labelled and named ci: {init}"
    );
    assert_eq!(init.matches("installdependencies.sh").count(), 1);
    assert!(
        init.find("installdependencies.sh") < init.find("config.sh"),
        "the libraries before either registers"
    );
    // each runner its own runcmd lines: one that fails to register leaves the other's lines to run
    assert!(init.lines().all(|l| l.matches("config.sh").count() <= 1));
}

/// Done-when 2 of hostman-ci-klar-runner: whichever runner is missing, the site_perso one or the klar-im one, is waited on
/// 15 minutes, then the box is reinstalled with a fresh token for each registration; under the one hourly limit and the
/// one alarm, whichever goes missing next.
#[dash_test]
fn whichever_runner_is_missing_reinstalls_the_box_with_a_token_each_under_one_hour_and_one_alarm() {
    let two = || Want {
        registrations: vec!["nha-org/site_perso".into(), "klar-im".into()],
        ..want()
    };
    for missing in ["klar-im", "nha-org/site_perso"] {
        let (call, sent) = hostman(ON, 30.0, 0.0);
        let gone = Rc::new(RefCell::new(missing));
        let g = Rc::clone(&gone);
        let (r, clock) = clocked(call, two(), Rc::new(RefCell::new(Ok(false))));
        let r = Runner {
            online: Box::new(move |reg: &Reg| Ok((reg.to != *g.borrow()).into())),
            ..r
        };
        let alarms = RefCell::new(vec![]);
        let tick = || {
            let rows = r.tick();
            alarms.borrow_mut().extend(text(&rows, "alarm"));
            clock.set(clock.get() + MIN);
            text(&rows, "status").unwrap()
        };
        for _ in 0..15 {
            let s = tick();
            assert_eq!(
                s,
                format!("waiting: server 7 is on, no runner online yet for {missing}")
            );
        }
        assert_eq!(
            tick(),
            format!(
                "reinstalled server 7 with a fresh registration token: no runner online for {missing}; {READ_7}"
            ),
            "{missing}"
        );
        assert_eq!(
            *sent.borrow(),
            [r#"PATCH /api/v1/servers/7 {"cloud_init":"token T1 T2","comment":"sha","os_id":141}"#],
            "a fresh token for each registration, whichever was missing"
        );
        // the other runner goes missing within the hour: no second reinstall, the same one alarm
        *gone.borrow_mut() = if missing == "klar-im" {
            "nha-org/site_perso"
        } else {
            "klar-im"
        };
        let statuses: Vec<String> = (0..44).map(|_| tick()).collect();
        assert!(
            statuses
                .last()
                .unwrap()
                .starts_with("down: server 7: no runner online, or in error, after its reinstall")
        );
        assert_eq!(sent.borrow().len(), 1, "one reinstall within the hour");
        assert!(
            (0..20)
                .map(|_| tick())
                .any(|s| s.starts_with("reinstalled")),
            "the next an hour after the last"
        );
        assert!(sent.borrow().iter().any(|w| w.contains("token T3 T4")));
        assert_eq!(
            alarms.borrow().len(),
            1,
            "one alarm whichever is missing: {alarms:?}"
        );
        *gone.borrow_mut() = "";
        assert_eq!(tick(), "up");
        assert_eq!(
            alarms.borrow().last().unwrap(),
            "",
            "cleared once both are up"
        );
    }
    // one registration's list unread never hides the other's missing runner (/code-review): site_perso missing, klar-im
    // unread, the box is reinstalled once its 15 minutes are up
    let (call, sent) = hostman(ON, 30.0, 0.0);
    let (r, clock) = clocked(call, two(), Rc::new(RefCell::new(Ok(false))));
    let r = Runner {
        online: Box::new(|reg: &Reg| match reg.to.as_str() {
            "klar-im" => Err("HTTP 502".into()),
            _ => Ok(Seen::Gone),
        }),
        ..r
    };
    let statuses: Vec<String> = (0..16)
        .map(|_| {
            clock.set(clock.get() + MIN);
            text(&r.tick(), "status").unwrap()
        })
        .collect();
    assert_eq!(
        statuses[0],
        "waiting: server 7 is on, no runner online yet for nha-org/site_perso"
    );
    assert_eq!(
        statuses[15],
        format!(
            "reinstalled server 7 with a fresh registration token: no runner online for nha-org/site_perso; {READ_7}"
        )
    );
    assert!(
        sent.borrow()[0].contains(r#""cloud_init":"token T1 T2""#),
        "{:?}",
        sent.borrow()
    );
    // a registration GitHub gives no token is left out, never the others with it (/code-review); none at all is Down
    let (call, sent) = hostman(NONE, 30.0, 0.0);
    let r = Runner::new(
        two(),
        call,
        Box::new(|_| Ok(Seen::Gone)),
        Box::new(|reg: &Reg| match reg.to.as_str() {
            "klar-im" => Err("HTTP 403".into()),
            _ => Ok("TA".into()),
        }),
    );
    assert_eq!(text(&r.tick(), "status").unwrap(), "created server 42");
    assert!(
        sent.borrow()[0].contains(r#""cloud_init":"token TA""#),
        "{:?}",
        sent.borrow()
    );
    let (call, sent) = hostman(NONE, 30.0, 0.0);
    let r = Runner::new(
        two(),
        call,
        Box::new(|_| Ok(Seen::Gone)),
        Box::new(|_| Err("HTTP 403".into())),
    );
    assert!(
        text(&r.tick(), "alarm")
            .unwrap()
            .contains("HTTP 403 (klar-im)")
    );
    assert!(sent.borrow().is_empty(), "no token, no create");
}

/// His OS (@nha, 2026-11-15: « debian. or alpine. », « don't pick ubuntu. »): the newest Debian listed. Hostman's list of
/// 2026-11-15 (Debian 11 and 12, Ubuntu 24.04) gives 12; a list with no Debian creates and wipes nothing; Ubuntu is refused
/// whatever the list holds.
#[dash_test]
fn the_os_is_the_newest_debian_listed_and_never_ubuntu() {
    assert_eq!(OS, "debian");
    assert_eq!(Want::eight_gb(&["o/r"]).os, OS);
    let live = r#"{"servers_os":[{"id":67,"family":"linux","name":"debian","version":"11"},{"id":99,"family":"linux","name":"ubuntu","version":"24.04"},{"id":95,"family":"linux","name":"debian","version":"12"}]}"#;
    let (call, sent) = hostman_with(NONE, live, 30.0, 0.0);
    let r = Runner::new(
        want(),
        call,
        Box::new(|_| Ok(Seen::Gone)),
        Box::new(|_| Ok("T".into())),
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
        Box::new(|_| Ok(Seen::Gone)),
        Box::new(|_| Ok("T".into())),
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
        Box::new(|_| Ok(Seen::Gone)),
        Box::new(|_| Ok("T".into())),
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
        Box::new(|_| Ok(Seen::Gone)),
        Box::new(|_| Ok("T".into())),
    );
    assert!(text(&r.tick(), "alarm").unwrap().contains("no alpine"));
    assert!(sent.borrow().is_empty());
}

/// Part 2: a server on with no runner online is waited on 15 minutes, then reinstalled with a fresh token (one PATCH of its
/// cloud-init and its os_id), at most once an hour; while it keeps failing, one alarm, never silence, and it clears when
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
        format!(
            "reinstalled server 7 with a fresh registration token: no runner online for nha-org/site_perso; {READ_7}"
        )
    );
    assert_eq!(
        *sent.borrow(),
        [r#"PATCH /api/v1/servers/7 {"cloud_init":"token T1","comment":"sha","os_id":141}"#]
    );
    // 15 more minutes with no runner: one alarm, and no second reinstall within the hour
    let statuses: Vec<String> = (0..44).map(|_| tick(&r)).collect();
    assert!(
        statuses
            .last()
            .unwrap()
            .starts_with("down: server 7: no runner online, or in error, after its reinstall")
    );
    assert_eq!(alarms.borrow().len(), 1, "{alarms:?}");
    assert_eq!(sent.borrow().len(), 1, "one reinstall within the hour");
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
            "server 7: no runner online, or in error, after its reinstall; the next is an hour after the last",
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
        Box::new(|_| Ok(Seen::Idle)),
        Box::new(|_| Ok(String::new())),
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

/// An IPv4 add the hostman tool refuses (its cap, as ci-2's on 2026-11-19) or that fails is one attempt: the status says why,
/// and no add is sent again for `HEAL_MS`, as a create the credit refused; then one more.
#[dash_test]
fn a_refused_ipv4_add_is_one_attempt_then_said_until_the_wait_is_past() {
    let bare = r#"{"servers":[{"id":7,"name":"ci-runner","status":"on","os":{"name":"debian","version":"13"},"networks":[{"bandwidth":200,"blocked_ports":[],"ips":[],"type":"public"}]}]}"#;
    let refusal = "hostman: refused: a public IPv4 on server 7 costs 2.00 USD a month, 32.00 USD a month with the servers \
                   running, past the spend cap of 26 USD of credit left";
    for answer in [refusal, "hostman: no answer: curl 28"] {
        let (inner, _) = hostman(bare, 30.0, 0.0);
        let adds = Rc::new(Cell::new(0));
        let n = Rc::clone(&adds);
        let call: Tool = Box::new(move |asked: &str| match asked.contains("/ips") {
            true => {
                n.set(n.get() + 1);
                Err(answer.to_string())
            }
            false => inner(asked),
        });
        let (r, clock) = clocked(call, want(), Rc::new(RefCell::new(Ok(false))));
        let first = text(&r.tick(), "status").unwrap();
        assert_eq!(first, format!("down: {answer}"));
        for _ in 0..10 {
            clock.set(clock.get() + MIN);
            assert_eq!(
                text(&r.tick(), "status").unwrap(),
                first,
                "said again, nothing sent"
            );
        }
        assert_eq!(adds.get(), 1, "one attempt within the wait");
        clock.set(clock.get() + HEAL_MS);
        r.tick();
        assert_eq!(adds.get(), 2, "one more past it");
    }
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
        Box::new(|_| Ok(Seen::Gone)),
        Box::new(|_| Ok("T".into())),
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
    assert_eq!(paths(&sent), ["PATCH /api/v1/servers/7"]);
}

/// Task hostman-ci-error: server 9602127 as Hostman listed it from 12:02 on 2026-11-15, after the deprecated « install »
/// action (a 204 with no body, read as « answered no JSON », so never counted): « error », a status Hostman's OpenAPI
/// does not list, which the reconcile read as Hostman at work and waited on in silence. Now it is reinstalled at once by a
/// PATCH of its os_id with a token for each registration, then one alarm while it stays in error within the hour; any
/// other unlisted status is an alarm, never a silent wait.
#[dash_test]
fn a_server_in_error_is_reinstalled_by_its_os_id_and_an_unlisted_status_alarms() {
    let error = r#"{"servers":[{"id":9602127,"name":"ci","status":"error","os":{"id":95,"name":"debian","version":"12"},"networks":[{"bandwidth":200,"ips":[{"ip":"195.216.168.166","type":"ipv4"}],"type":"public"}]}]}"#;
    let (call, sent) = hostman(error, 29.72, 22.0);
    let (r, clock) = clocked(
        call,
        Want::eight_gb(&["nha-org/site_perso", "klar-im"]),
        Rc::new(RefCell::new(Ok(false))),
    );
    // a box in error asks GitHub nothing: its answer changes nothing (/code-review)
    let r = Runner {
        online: Box::new(|reg: &Reg| panic!("GitHub asked about {reg} for a box in error")),
        ..r
    };
    let rows = r.tick();
    assert_eq!(
        text(&rows, "status").unwrap(),
        "reinstalled server 9602127 with a fresh registration token: it was in error; no reading listed"
    );
    assert_eq!(
        *sent.borrow(),
        [
            r#"PATCH /api/v1/servers/9602127 {"cloud_init":"token T1 T2","comment":"sha","os_id":141}"#
        ],
        "one PATCH of its os_id, never the install action"
    );
    let alarms: Vec<String> = (0..30)
        .filter_map(|_| {
            clock.set(clock.get() + MIN);
            text(&r.tick(), "alarm")
        })
        .collect();
    assert_eq!(
        alarms,
        [
            "server 9602127: no runner online, or in error, after its reinstall; the next is an hour after the last"
        ]
    );
    assert_eq!(sent.borrow().len(), 1, "one reinstall within the hour");

    let (r, sent) = runner(
        r#"{"servers":[{"id":7,"name":"ci-runner","status":"removed","os":{"name":"debian","version":"13"}}]}"#,
        false,
        30.0,
    );
    assert!(
        text(&r.tick(), "alarm")
            .unwrap()
            .contains("server 7 is removed, which Hostman does not list as at work")
    );
    assert!(sent.borrow().is_empty());
}

/// The coordinator's three of 2026-11-15 (@nha: « site-perso-ci => just ci. »): server 9602127, site-perso-ci on Ubuntu, is
/// renamed ci and reinstalled on Debian 13 in one PATCH (its os_id reinstalls it), once, with
/// a fresh token, and never a create beside it.
#[dash_test]
fn the_old_ubuntu_box_is_renamed_and_reinstalled_on_debian_never_a_second() {
    let old = r#"{"servers":[{"id":9602127,"name":"site-perso-ci","status":"on","os":{"id":99,"name":"ubuntu","version":"24.04"},"networks":[{"ips":[{"ip":"5.6.7.8","type":"ipv4"}],"type":"public"}]}]}"#;
    let (call, sent) = hostman(old, 30.0, 20.0);
    let (r, clock) = clocked(
        call,
        Want::eight_gb(&["nha-org/site_perso"]),
        Rc::new(RefCell::new(Ok(false))),
    );
    assert_eq!(
        text(&r.tick(), "status").unwrap(),
        "reinstalled server 9602127 with a fresh registration token: it was named site-perso-ci; no reading listed"
    );
    assert_eq!(
        *sent.borrow(),
        [
            r#"PATCH /api/v1/servers/9602127 {"cloud_init":"token T1","comment":"sha","name":"ci","os_id":141}"#
        ]
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
        Step::Reinstalled(7, "it was in error".into()).text(),
        "waiting: server 7 is installing".into(),
        Step::Waiting(7, String::new()).text(),
        Step::Waiting(7, String::new()).text(),
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
    let want = Want::eight_gb(&["nha-org/site_perso"]);
    let state = seed(&log, at(4), &want).unwrap();
    assert_eq!(state.installed, Some(at(1)), "{state:?}");
    assert_eq!(
        state.since,
        Some(at(3)),
        "the run of no runner began after the install: {state:?}"
    );
    let later = seed(&log, at(4) + REINSTALL_MS + MIN, &want).unwrap();
    assert_eq!(later, State::default(), "past the hour, nothing kept");
}

/// Part 3: the month's cost against the credit left, checked before each create: past it, no POST is sent and the alarm
/// names both numbers.
#[dash_test]
fn a_create_past_the_credit_left_is_refused_before_any_write() {
    let (r, sent) = runner(NONE, false, 15.0);
    let alarm = text(&r.tick(), "alarm").unwrap();
    assert!(
        alarm.contains("costs 20.00 a month") && alarm.contains("past the 15 of credit left"),
        "{alarm}"
    );
    assert!(sent.borrow().is_empty(), "nothing was created");

    // the servers already running count: 20 + 10 running is past 25
    let (call, sent) = hostman(NONE, 25.0, 10.0);
    let r = Runner::new(
        want(),
        call,
        Box::new(|_| Ok(Seen::Gone)),
        Box::new(|_| Ok("T".into())),
    );
    assert!(
        text(&r.tick(), "alarm")
            .unwrap()
            .contains("30.00 with the servers running")
    );
    assert!(sent.borrow().is_empty());

    // and exactly the credit is allowed: the IPv4 is the hostman tool's to price, on the create it refuses or passes
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
    let want = Want::eight_gb(&["nha-org/site_perso"]);
    assert_eq!(want.ram_mb, 8192);
    let live: Json = serde_json::from_str(LIVE).unwrap();
    let order: Vec<i64> = (candidates(&live, &want, &[]).iter())
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
            Box::new(|_| Ok(Seen::Gone)),
            Box::new(|_| Ok("T".into())),
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
    let on = |listed: &str| online(listed, "ci", LABEL);
    assert_eq!(on(&listed("online", "ci", LABEL)), Ok(Seen::Idle));
    assert_eq!(on(&listed("offline", "ci", LABEL)), Ok(Seen::Gone));
    assert_eq!(
        on(&listed("online", "ci", "site-perso-vps")),
        Ok(Seen::Gone)
    );
    assert!(on("not json").is_err());
    // what GitHub listed on 2026-11-15: the whatsnr VPS runners carry the label as a bridge, and are not this box's runner
    let bridge = r#"{"runners":[{"name":"whatsnr-prod-1-site-perso-vps","status":"online","labels":[{"name":"site-perso-vps"},{"name":"site-perso-hostman"},{"name":"ci"}]}]}"#;
    assert_eq!(on(bridge), Ok(Seen::Gone));
    let mut log = dash_core::MemoryLog::default();
    let (r, _) = runner(ON, true, 30.0);
    log.write(&r.tick()).unwrap();
    let at = log.0[0].stamp >> 16;
    assert_eq!(
        status(&log, at, "ci-runner").unwrap(),
        Some(("up".into(), at))
    );
    assert_eq!(
        status(&log, at + STALE_MS + 1, "ci-runner").unwrap(),
        None,
        "an old status is no reading"
    );
    let big = &super::live::pool()[0];
    let up = lines(big, Ok(Some(("up".into(), at))), at + 3_000);
    assert_eq!(up[0], "State: ok");
    assert_eq!(
        up[1],
        "Box: Hostman Cloud Server ci, 8 GB, a runner each for nha-org/site_perso under ci, klar-im under ci, \
         nha-org/site_perso under ci-light, kept up each minute"
    );
    assert_eq!(up[2], "Status: up (3 s ago)");
    assert_eq!(up[3], "Alarm: none");
    let down = lines(
        big,
        Ok(Some(("down: server 7 is blocked: his".into(), at))),
        at,
    );
    assert_eq!(
        (down[0].as_str(), down[3].as_str()),
        ("State: NOT OK", "Alarm: server 7 is blocked: his")
    );
    let silent = lines(big, Ok(None), at);
    assert_eq!(silent[0], "State: NOT OK");
    assert!(silent[3].contains("the job is not running"), "{silent:?}");
}

/// Done-when 1 of ci-box-open-and-current: root takes his keys, read at run time from ~/.ssh/id_*.pub (a deploy key beside
/// them is left out), quoted so a key's comment never breaks the YAML; none read is Down, never a box remade with no key.
#[dash_test]
fn root_takes_his_ssh_keys_read_from_his_home_at_run_time() {
    use super::live::keys;
    let home = std::env::temp_dir().join(format!("hostman-ci-keys-{}", std::process::id()));
    let ssh = home.join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    std::fs::write(
        ssh.join("id_ed25519.pub"),
        "ssh-ed25519 AAAAC3k nicolas@mac: \"home\"\n",
    )
    .unwrap();
    std::fs::write(ssh.join("id_ed25519"), "PRIVATE").unwrap();
    std::fs::write(
        ssh.join("graal-ci-deploy.pub"),
        "ssh-ed25519 AAAAdeploy graal-ci\n",
    )
    .unwrap();
    let read = keys(&home).unwrap();
    assert_eq!(read, ["ssh-ed25519 AAAAC3k nicolas@mac: \"home\""]);
    let init = init(&[("nha-org/site_perso", "T".into())], &read);
    assert!(init.contains("\ndisable_root: false\n"), "{init}");
    assert!(
        init.contains(r#"ssh_authorized_keys: ["ssh-ed25519 AAAAC3k nicolas@mac: \"home\""]"#),
        "{init}"
    );
    std::fs::remove_file(ssh.join("id_ed25519.pub")).unwrap();
    assert!(keys(&home).unwrap_err().contains("no public key"));

    let (call, sent) = hostman(NONE, 30.0, 0.0);
    let r = runner_with(call, |_| Ok(Seen::Gone)).keyed(Box::new(move || keys(&home)));
    let rows = r.tick();
    assert!(
        text(&rows, "status")
            .unwrap()
            .starts_with("down: no ssh key for root: no public key")
    );
    assert!(
        sent.borrow().is_empty(),
        "nothing created with no key: {:?}",
        sent.borrow()
    );
}

/// Done-when 2: unattended-upgrades is installed and apt's daily timers run it, its file written after the packages.
#[dash_test]
fn security_updates_apply_daily_by_unattended_upgrades() {
    let init = init(&[("nha-org/site_perso", "T".into())], &[]);
    let packages = init.lines().find(|l| l.starts_with("packages: ")).unwrap();
    assert!(packages.contains("unattended-upgrades"), "{packages}");
    assert!(
        init.contains(
            "  - path: /etc/apt/apt.conf.d/20auto-upgrades\n    defer: true\n    content: \"APT::Periodic::Update-Package-Lists \\\"1\\\";\\nAPT::Periodic::Unattended-Upgrade \\\"1\\\";\\n\"\n"
        ),
        "{init}"
    );
}

/// Done-when 4: Caddy, installed with the packages, answers whatsnr.eu, http and https, with a 301 to the same path on
/// whatsnr.com, its Caddyfile written after the package and the service restarted on it.
#[dash_test]
fn caddy_sends_whatsnr_eu_to_the_same_path_on_whatsnr_com() {
    let init = init(&[("nha-org/site_perso", "T".into())], &[]);
    assert!(
        init.lines()
            .any(|l| l.starts_with("packages: ") && l.contains(", caddy]"))
    );
    assert_eq!(
        CADDYFILE,
        "http://whatsnr.eu, whatsnr.eu {\n\tredir https://whatsnr.com{uri} permanent\n}\n"
    );
    assert!(init.contains(&format!(
        "  - path: /etc/caddy/Caddyfile\n    defer: true\n    content: {}\n",
        Json::from(CADDYFILE)
    )));
    assert!(
        init.contains("runcmd:\n  - systemctl restart caddy docker\n"),
        "{init}"
    );
}

fn runner_with(call: Tool, seen: impl Fn(&Reg) -> Result<Seen, String> + 'static) -> Runner {
    Runner::new(
        want(),
        call,
        Box::new(seen),
        Box::new(|_| Ok("TOKEN".into())),
    )
}

/// Done-when 3: a box whose comment is not this cloud-init's hash is reinstalled, the new hash its comment, only once its
/// runners are idle (up meanwhile, so CI still routes to it); the same hash, or no comment read, wipes nothing.
#[dash_test]
fn a_changed_cloud_init_reinstalls_the_box_only_while_its_runners_are_idle() {
    let keys = vec!["ssh-ed25519 AAAAC3k nicolas@mac".to_string()];
    let print = fingerprint(&want(), &keys);
    assert!(
        print.starts_with("cloud-init sha256 ") && print.len() == 18 + 64,
        "{print}"
    );
    let other = vec!["ssh-ed25519 AAAAnew nicolas@mac".to_string()];
    assert_ne!(
        print,
        fingerprint(&want(), &other),
        "a new key is a new cloud-init"
    );
    let server = |comment: &str| -> &'static str {
        let s = format!(
            r#"{{"servers":[{{"id":7,"name":"ci-runner","status":"on","comment":"{comment}","os":{{"name":"debian","version":"13"}}}}]}}"#
        );
        Box::leak(s.into_boxed_str())
    };
    let busy = Rc::new(Cell::new(true));
    let b = Rc::clone(&busy);
    let (call, sent) = hostman(server("cloud-init sha256 old"), 30.0, 0.0);
    let k = keys.clone();
    let r = runner_with(call, move |_| {
        Ok(if b.get() { Seen::Busy } else { Seen::Idle })
    })
    .keyed(Box::new(move || Ok(k.clone())));
    let held = r.tick();
    assert_eq!(
        text(&held, "status").unwrap(),
        "up: server 7 runs an older cloud-init; it is reinstalled once its runners are idle, an hour after the last"
    );
    assert!(sent.borrow().is_empty(), "a busy runner is never wiped");
    busy.set(false);
    assert_eq!(
        text(&r.tick(), "status").unwrap(),
        format!(
            "reinstalled server 7 with a fresh registration token: its cloud-init changed; {READ_7}"
        )
    );
    let patch: Json =
        serde_json::from_str(sent.borrow()[0].splitn(3, ' ').nth(2).unwrap()).unwrap();
    assert_eq!(patch["os_id"], 141);
    assert!(
        sent.borrow()[0].contains(r#""comment":"sha""#),
        "{:?}",
        sent.borrow()
    );

    for listed in [server(&print), ON] {
        let (call, sent) = hostman(listed, 30.0, 0.0);
        let k = keys.clone();
        let r = runner_with(call, |_| Ok(Seen::Idle)).keyed(Box::new(move || Ok(k.clone())));
        assert_eq!(text(&r.tick(), "status").unwrap(), "up");
        assert!(sent.borrow().is_empty());
    }

    // a missing runner's heal waits for the other's job: past its 15 minutes, nothing sent while one is busy
    let (call, sent) = hostman(ON, 30.0, 0.0);
    let r = runner_with(call, |reg| {
        Ok(if reg.to == "klar-im" {
            Seen::Gone
        } else {
            Seen::Busy
        })
    });
    let r = Runner {
        want: Want {
            registrations: vec!["nha-org/site_perso".into(), "klar-im".into()],
            ..want()
        },
        ..r
    };
    let clock = Rc::new(Cell::new(1_000_000));
    let now = Rc::clone(&clock);
    let r = r.clocked(Box::new(move || {
        now.set(now.get() + MIN);
        now.get()
    }));
    for _ in 0..30 {
        assert_eq!(
            text(&r.tick(), "status").unwrap(),
            "waiting: server 7 is on, no runner online yet for klar-im"
        );
    }
    assert!(sent.borrow().is_empty(), "{:?}", sent.borrow());
}

/// GitHub's list read busy: a runner online and at a job is `Busy`, which no reinstall wipes.
#[dash_test]
fn github_s_list_reads_a_busy_runner() {
    use super::live::online;
    let listed = |busy: bool| {
        format!(
            r#"{{"runners":[{{"name":"ci","status":"online","busy":{busy},"labels":[{{"name":"ci"}}]}}]}}"#
        )
    };
    assert_eq!(online(&listed(true), "ci", LABEL), Ok(Seen::Busy));
    assert_eq!(online(&listed(false), "ci", LABEL), Ok(Seen::Idle));
}

/// Done-when 1's <ip>: each tick writes the server's public IPv4, and /status/ci-runner's last line says `ssh root@` it.
#[dash_test]
fn the_status_page_names_the_box_s_address_for_ssh() {
    use super::live::{STALE_MS, address};
    use dash_core::OpLog;
    let listed = r#"{"servers":[{"id":7,"name":"ci-runner","status":"on","os":{"name":"debian","version":"13"},"networks":[{"type":"public","ips":[{"type":"ipv6","ip":"2a03::1"},{"type":"ipv4","ip":"203.0.113.7"}]}]}]}"#;
    let (r, _) = runner(listed, true, 30.0);
    let mut log = dash_core::MemoryLog::default();
    log.write(&r.tick()).unwrap();
    let at = log.0[0].stamp >> 16;
    assert_eq!(
        address(&log, at, "ci-runner").unwrap(),
        "Address: ssh root@203.0.113.7"
    );
    assert_eq!(
        address(&log, at, "ci-2").unwrap(),
        "Address: none listed in the last ticks",
        "another box's row"
    );
    assert_eq!(
        address(&log, at + STALE_MS + 1, "ci-runner").unwrap(),
        "Address: none listed in the last ticks"
    );
}

/// /code-review of ci-box-open-and-current: a comment Hostman does not keep is one reinstall and an alarm, never one an
/// hour; a key unread blocks no start; a busy box with a list unread is held, then the alarm it was.
#[dash_test]
fn a_comment_hostman_drops_is_one_reinstall_and_an_alarm_never_a_loop() {
    const OLD: &str = r#"{"servers":[{"id":7,"name":"ci-runner","status":"on","comment":"cloud-init sha256 old","os":{"name":"debian","version":"13"}}]}"#;
    let (call, sent) = hostman(OLD, 30.0, 0.0);
    let clock = Rc::new(Cell::new(1_000_000));
    let now = Rc::clone(&clock);
    let r = runner_with(call, |_| Ok(Seen::Idle))
        .keyed(Box::new(|| Ok(vec!["ssh-ed25519 K".into()])))
        .clocked(Box::new(move || {
            now.set(now.get() + MIN);
            now.get()
        }));
    let rows: Vec<Vec<FieldWrite>> = (0..180).map(|_| r.tick()).collect();
    assert_eq!(
        paths(&sent),
        ["PATCH /api/v1/servers/7"],
        "one reinstall in three hours"
    );
    let alarms: Vec<String> = rows.iter().filter_map(|w| text(w, "alarm")).collect();
    assert_eq!(
        alarms,
        [
            "server 7: its comment is not the cloud-init hash its reinstall sent, so Hostman keeps no comment and no other reinstall follows"
        ]
    );

    let (call, sent) = hostman(OFF, 30.0, 0.0);
    let r = runner_with(call, |_| Ok(Seen::Gone)).keyed(Box::new(|| Err("denied".into())));
    assert_eq!(text(&r.tick(), "status").unwrap(), "started server 7");
    assert_eq!(paths(&sent), ["POST /api/v1/servers/7/start"]);
}

#[dash_test]
fn a_busy_box_never_reads_no_runner_and_an_unread_list_still_alarms() {
    const WAS: &str = r#"{"servers":[{"id":7,"name":"site-perso-ci","status":"on","os":{"name":"debian","version":"13"}}]}"#;
    let (call, sent) = hostman(WAS, 30.0, 0.0);
    let r = runner_with(call, |_| Ok(Seen::Busy));
    let r = Runner {
        want: Want {
            was: vec!["site-perso-ci".into()],
            ..want()
        },
        ..r
    };
    assert!(
        text(&r.tick(), "status")
            .unwrap()
            .starts_with("up: server 7"),
        "held, never « no runner »"
    );
    assert!(sent.borrow().is_empty());

    let (call, sent) = hostman(ON, 30.0, 0.0);
    let r = runner_with(call, |reg| match reg.to.as_str() {
        "klar-im" => Err("HTTP 502".into()),
        _ => Ok(Seen::Busy),
    });
    let r = Runner {
        want: Want {
            registrations: vec!["nha-org/site_perso".into(), "klar-im".into()],
            ..want()
        },
        ..r
    };
    let clock = Rc::new(Cell::new(1_000_000));
    let now = Rc::clone(&clock);
    let r = r.clocked(Box::new(move || {
        now.set(now.get() + MIN);
        now.get()
    }));
    let statuses: Vec<String> = (0..16)
        .map(|_| text(&r.tick(), "status").unwrap())
        .collect();
    assert!(
        statuses[..15].iter().all(|s| s.starts_with("up: server 7")),
        "{statuses:?}"
    );
    assert_eq!(
        statuses[15],
        "down: server 7: GitHub's runner list cannot be read: HTTP 502 (klar-im)"
    );
    assert!(sent.borrow().is_empty());
}

/// The box died twice on 2026-11-18, each time mid cold build on 8 GB with no swap (16:22, and 17:13 in `cargo clippy` on the
/// freshly reinstalled box, run 44327318496): the runner gone with no log. Each box swaps 4 GB, on from the first boot and at
/// each boot after (fstab), and each runner's environment caps cargo's jobs at its box's `jobs`: 3 on the 8 GB box. A
/// changed cloud-init moves the fingerprint, so the running box takes it at its own reinstall, once idle (task
/// ci-box-open-and-current), never by hand.
#[dash_test]
fn each_box_swaps_4_gb_and_caps_cargo_s_jobs_by_its_size() {
    let init = init(&[("nha-org/site_perso", "T".into())], &[]);
    assert!(
        init.contains(&format!(
            "runcmd:\n  - systemctl restart caddy docker\n  - {SWAP}\n"
        )),
        "{init}"
    );
    assert_eq!(
        SWAP,
        "fallocate -l 4G /swapfile && chmod 600 /swapfile && mkswap /swapfile && swapon /swapfile && echo '/swapfile none swap sw 0 0' >> /etc/fstab"
    );
    assert!(
        init.contains("echo NEXTEST_TEST_THREADS=2 >> .env && echo CARGO_BUILD_JOBS=3 >> .env"),
        "{init}"
    );
    let big = Want::eight_gb(&["o/r"]);
    assert_eq!(big.jobs, 3);
    assert_ne!(
        fingerprint(&big, &[]),
        fingerprint(
            &Want {
                jobs: 2,
                ..big.clone()
            },
            &[]
        ),
        "a box running the old cloud-init is reinstalled by the reconciler"
    );
}

/// apt waits for its lock, written before the packages; dockerd keeps its containers across an update's restart.
#[dash_test]
fn apt_waits_for_its_lock_and_docker_keeps_its_containers() {
    let init = init(&[("nha-org/site_perso", "T".into())], &[]);
    assert!(init.contains("write_files:\n  - path: /etc/apt/apt.conf.d/90lock-wait\n    content: \"DPkg::Lock::Timeout \\\"600\\\";\\n\"\n  - path:"), "{init}");
    assert!(init.contains("  - path: /etc/docker/daemon.json\n    defer: true\n    content: \"{\\\"live-restore\\\": true}\\n\"\n"), "{init}");
    assert!(init.contains("runcmd:\n  - systemctl restart caddy docker\n"));
}

/// What a reinstall's status says of `STATS`: server 7's newest readings, with no disk listed.
const READ_7: &str = "last readings: cpu 97%, ram 7801 of 7953 MB at 2026-11-18T15:55:41.000Z";
/// What Hostman read of server 7, as its statistics answered for server 9608429 on 2026-11-18: newest first, UTC, a point
/// every five minutes at best (CPU load from 0 to 1, RAM in MB); the newest a box short of memory under a cold build.
const STATS: &str = r#"{"cpu":[{"logged_at":"2026-11-18T15:55:41.000Z","load":0.97},{"logged_at":"2026-11-18T15:51:05.000Z","load":0.5}],"ram":[{"logged_at":"2026-11-18T15:55:41.000Z","total":7953.285,"used":7801.4,"used_cached":7740.2,"available":40.1},{"logged_at":"2026-11-18T11:38:37.000Z","total":7953.285,"used":1803.086,"used_cached":7701.781,"available":6150.199}],"disk":[{"logged_at":"2026-11-18T15:55:41.000Z","read":0.002,"write":0}],"network_traffic":[]}"#;
/// Server 7 on, its system disk nearly full, as GET /api/v1/servers lists its disks (`used` and `size` in MB).
const FULL: &str = r#"{"servers":[{"id":7,"name":"ci-runner","status":"on","os":{"name":"debian","version":"13"},"disks":[{"size":20480,"used":1,"is_system":false},{"size":163840,"used":150000,"is_system":true}]}]}"#;

/// 2026-11-18 (task ci-runner-pool): the box froze at 16:22 mid-build, its runner vanished, and the reconcile waited the
/// hour since its last reinstall before the next, the reason gone with the disk. A runner that WAS online since the last
/// reinstall and vanished, its box on, is healed after 15 minutes whatever that hour says; a fresh install keeps its hour
/// for cloud-init. Before any reinstall, Hostman's last readings of the box (CPU, RAM, its system disk) and the reason go in
/// its status, and the heal's alarm says them, cleared once it is up again.
#[dash_test]
fn a_runner_that_was_online_and_vanished_is_healed_after_15_minutes_its_last_readings_kept() {
    let (call, sent) = hostman(FULL, 30.0, 0.0);
    let seen = Rc::new(RefCell::new(Ok(false)));
    let (r, clock) = clocked(call, want(), Rc::clone(&seen));
    let alarms = RefCell::new(vec![]);
    let tick = || {
        let rows = r.tick();
        alarms.borrow_mut().extend(text(&rows, "alarm"));
        clock.set(clock.get() + MIN);
        text(&rows, "status").unwrap()
    };
    // a fresh box: its 15 minutes, a reinstall for a runner never seen, which is no alarm
    (0..15).for_each(|_| assert!(tick().starts_with("waiting")));
    let first = tick();
    assert_eq!(
        first,
        format!(
            "reinstalled server 7 with a fresh registration token: no runner online for nha-org/site_perso; {READ_7}, \
             disk 150000 of 163840 MB used"
        )
    );
    *seen.borrow_mut() = Ok(true);
    (0..5).for_each(|_| assert_eq!(tick(), "up"));
    // 21 minutes after that reinstall its runner vanishes, the box still on: healed 15 minutes on, inside the hour
    *seen.borrow_mut() = Ok(false);
    let statuses: Vec<String> = (0..16).map(|_| tick()).collect();
    assert!(
        statuses[..15].iter().all(|s| s.starts_with("waiting")),
        "{statuses:?}"
    );
    let healed = format!(
        "reinstalled server 7 with a fresh registration token: its runner for nha-org/site_perso vanished after it was \
         online; {READ_7}, disk 150000 of 163840 MB used"
    );
    assert_eq!(statuses[15], healed);
    assert_eq!(sent.borrow().len(), 2, "{:?}", sent.borrow());
    assert_eq!(
        *alarms.borrow(),
        [healed],
        "the heal's alarm says why and what the box read"
    );
    // its runner never comes back: the fresh install's hour holds, one alarm, no third reinstall within it
    let after: Vec<String> = (0..40).map(|_| tick()).collect();
    assert!(
        after
            .last()
            .unwrap()
            .starts_with("down: server 7: no runner online"),
        "{after:?}"
    );
    assert_eq!(sent.borrow().len(), 2, "one heal, then the hour");
    *seen.borrow_mut() = Ok(true);
    assert_eq!(tick(), "up");
    assert_eq!(alarms.borrow().last().unwrap(), "", "cleared once it is up");
    // readings Hostman does not answer never hold the reinstall off: said unread
    let (call, sent) = hostman_with(FULL, OSES, 30.0, 0.0);
    let unread: Tool = Box::new(move |asked: &str| match asked.contains("/statistics?") {
        true => Err("hostman: no answer: it answered 500".into()),
        false => call(asked),
    });
    let r = runner_with(unread, |_| Ok(Seen::Gone)).seeded(State {
        since: Some(-HEAL_MS),
        ..State::default()
    });
    assert_eq!(
        text(&r.tick(), "status").unwrap(),
        "reinstalled server 7 with a fresh registration token: no runner online for nha-org/site_perso; last readings: \
         cpu and ram unread: hostman: no answer: it answered 500, disk 150000 of 163840 MB used"
    );
    assert_eq!(paths(&sent), ["PATCH /api/v1/servers/7"]);
}

/// A restart resumes which runners were seen online since the last reinstall, from the job's own statuses: « up » after it
/// is every registration's, so a deploy between a runner's death and its heal keeps the 15 minutes.
#[dash_test]
fn a_restart_resumes_the_runners_seen_since_the_last_reinstall() {
    use super::live::seed;
    let at = |i: i64| 10_000_000 + i * MIN;
    let log = |statuses: &[String]| {
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
        dash_core::MemoryLog(ops.collect(), false)
    };
    let want = Want::eight_gb(&["nha-org/site_perso", "klar-im"]);
    // an « up » says nothing of a runner added since it was written (ci-light, at this branch's first deploy): a restart
    // resumes no runner as seen, so its first death after a restart waits the hour, never a false heal (/code-review)
    let up = log(&[
        Step::Reinstalled(7, "its cloud-init changed".into()).text(),
        "up".into(),
        Step::Waiting(7, String::new()).text(),
    ]);
    let state = seed(&up, at(2), &want).unwrap();
    assert!(state.seen.is_empty(), "{state:?}");
    assert_eq!((state.installed, state.since), (Some(at(0)), Some(at(2))));
    // a reinstall's reason may say « no runner online for »: the wait starts at the first wait after it (/code-review)
    let said = log(&[
        Step::Reinstalled(7, "no runner online for klar-im".into()).text(),
        Step::Waiting(7, String::new()).text(),
    ]);
    assert_eq!(seed(&said, at(1), &want).unwrap().since, Some(at(1)));
    // the last heal is resumed as the last reinstall is: a restart never lets a second heal in within the hour
    let healed = log(&[
        Step::Healed(
            7,
            "its runner for klar-im vanished after it was online".into(),
        )
        .text(),
        "up".into(),
    ]);
    let state = seed(&healed, at(1), &want).unwrap();
    assert_eq!((state.healed, state.installed), (Some(at(0)), Some(at(0))));
    assert_eq!(
        seed(&up, at(2), &want).unwrap().healed,
        None,
        "a reinstall is no heal"
    );
}

/// Hostman's 4 GB presets as the hostman tool listed them on 2026-11-18, out of order: pl-1's at 6 USD has 100 Mbps and
/// 3 GHz, the others 200 Mbps at 8.
const LIVE_4: &str = r#"{"server_presets":[
{"id":6271,"location":"de-1","price":8,"cpu":2,"ram":4096,"disk":81920,"bandwidth":200},
{"id":5135,"location":"pl-1","price":6,"cpu":2,"ram":4096,"disk":81920,"bandwidth":100},
{"id":5513,"location":"us-3","price":8,"cpu":2,"ram":4096,"disk":81920,"bandwidth":200},
{"id":5159,"location":"nl-1","price":8,"cpu":2,"ram":4096,"disk":81920,"bandwidth":200},
{"id":5147,"location":"us-2","price":8,"cpu":2,"ram":4096,"disk":81920,"bandwidth":200},
{"id":5157,"location":"nl-1","price":20,"cpu":4,"ram":8192,"disk":163840,"bandwidth":200}]}"#;

/// The pool (task ci-runner-pool; @nha, 2026-11-18: « we can probably spin up a second server. I'd say this one "2 x 3.3
/// GHz, 4 GB, 80 GB, 200 Mbps, €8.00/mo" », « I suggest spinning the second server and seeing how it holds. »): the 8 GB
/// box ci as before, plus a light runner for this repository labelled ci-light alone, so a check that builds nothing never
/// waits behind a 30-minute build; and a 4 GB box ci-2, its one runner for this repository labelled ci, its cargo on two
/// jobs, its preset one of 200 Mbps from the live list, cheapest first, never pl-1's 100.
#[dash_test]
fn the_pool_is_the_8_gb_box_with_a_light_runner_and_a_4_gb_box() {
    use super::live::{REGISTRATIONS, pool};
    let [big, small] = &pool()[..] else {
        panic!("two boxes")
    };
    let said =
        |w: &Want| -> Vec<String> { w.registrations.iter().map(|r| r.to_string()).collect() };
    assert_eq!((big.name.as_str(), big.ram_mb, big.jobs), ("ci", 8192, 3));
    assert_eq!(
        said(big),
        [
            "nha-org/site_perso",
            "klar-im",
            "nha-org/site_perso (ci-light)"
        ]
    );
    assert_eq!(
        (small.name.as_str(), small.ram_mb, small.jobs, small.mbps),
        ("ci-2", 4096, 2, 200)
    );
    assert_eq!(said(small), [REGISTRATIONS[0]]);
    let live: Json = serde_json::from_str(LIVE_4).unwrap();
    let order: Vec<i64> = (candidates(&live, small, &[]).iter())
        .map(|p| p["id"].as_i64().unwrap())
        .collect();
    assert_eq!(order, [5147, 5159, 5513, 6271]);

    fn tokens(w: &Want) -> Vec<(&Reg, String)> {
        w.registrations
            .iter()
            .map(|r| (r, "T".to_string()))
            .collect()
    }
    let init = cloud_init(big, &tokens(big), &[]);
    for (dir, runner) in [
        (
            "/home/runner/nha-org-site_perso-ci",
            "--labels ci,ci-light --name ci --url https://github.com/nha-org/site_perso ",
        ),
        (
            "/home/runner/klar-im-ci",
            "--labels ci,ci-light --name ci --url https://github.com/klar-im ",
        ),
        (
            "/home/runner/nha-org-site_perso-ci-light",
            "--labels ci-light --name ci-light --url https://github.com/nha-org/site_perso ",
        ),
    ] {
        assert!(
            init.contains(&format!("cd {dir} && ./svc.sh install runner")),
            "{init}"
        );
        assert!(init.contains(runner), "{runner}: {init}");
    }
    let init = cloud_init(small, &tokens(small), &[]);
    assert!(init.contains(&format!("  - {SWAP}\n")) && init.contains("CARGO_BUILD_JOBS=2"));
    assert!(
        init.contains(
            "--labels ci,ci-light --name ci-2 --url https://github.com/nha-org/site_perso "
        )
    );
    assert_eq!(init.matches("config.sh").count(), 1, "{init}");
}

/// GitHub's list reads each runner by its name and its label: the light runner is ci-light under ci-light, never the ci
/// runner, and each box's statuses are its own rows.
#[dash_test]
fn each_runner_is_read_by_its_name_and_label_and_each_box_by_its_rows() {
    use super::live::{online, status};
    use dash_core::OpLog;
    let listed = r#"{"runners":[{"name":"ci","status":"online","busy":true,"labels":[{"name":"ci"}]},{"name":"ci-light","status":"online","busy":false,"labels":[{"name":"ci-light"}]}]}"#;
    assert_eq!(online(listed, "ci", LABEL), Ok(Seen::Busy));
    assert_eq!(online(listed, "ci-light", LIGHT), Ok(Seen::Idle));
    assert_eq!(online(listed, "ci-light", LABEL), Ok(Seen::Gone));
    assert_eq!(online(listed, "ci-2", LABEL), Ok(Seen::Gone));
    let mut log = dash_core::MemoryLog::default();
    let (r, _) = runner(ON, true, 30.0);
    log.write(&r.tick()).unwrap();
    let at = log.0[0].stamp >> 16;
    assert_eq!(
        status(&log, at, "ci-runner").unwrap(),
        Some(("up".into(), at))
    );
    assert_eq!(status(&log, at, "ci-2").unwrap(), None, "another box's row");
}

/// The 4 GB box's create, refused by the cap on 2026-11-18 (27.22 USD of credit left, 24.00 a month running): the credit
/// drains each hour, so a reason holding its cents was a new alarm every 20 minutes or so. The credit left is said in whole
/// units: one alarm while the refusal stands, another only when a whole unit has gone.
#[dash_test]
fn a_create_the_credit_refuses_is_one_alarm_while_the_credit_drains() {
    let left = Rc::new(Cell::new(27.22));
    let l = Rc::clone(&left);
    let (inner, sent) = hostman(NONE, 0.0, 0.0);
    let reads = Rc::new(Cell::new(0));
    let n = Rc::clone(&reads);
    let call: Tool = Box::new(move |asked: &str| {
        if !asked.contains(r#""path":"/api/v1/servers""#) {
            n.set(n.get() + 1);
        }
        match asked.contains("/api/v1/account/finances") {
            true => Ok(format!(
                r#"{{"finances":{{"balance":{},"monthly_cost":24}}}}"#,
                l.get()
            )),
            false => inner(asked),
        }
    });
    let clock = Rc::new(Cell::new(0));
    let now = Rc::clone(&clock);
    let r = runner_with(call, |_| Ok(Seen::Gone)).clocked(Box::new(move || now.get()));
    let mut alarms = vec![];
    for _ in 0..20 {
        alarms.extend(text(&r.tick(), "alarm"));
        left.set(left.get() - 0.01);
        clock.set(clock.get() + MIN / 2);
    }
    assert_eq!(
        alarms,
        [
            "no server created: preset 5157 costs 20.00 a month, 44.00 with the servers running, past the 27 of credit left"
        ]
    );
    assert!(sent.borrow().is_empty());
    // a refusal stands its 15 minutes: the OS list, the presets and the finances are read once in ten minutes, not each
    // minute (/code-review: 5700 audit rows a day)
    assert_eq!(reads.get(), 3, "os, presets, finances once");
    clock.set(HEAL_MS);
    r.tick();
    assert_eq!(reads.get(), 6, "asked again after 15 minutes");
}

/// A runner that comes up, then vanishes, its box on: the box died under it.
fn died(
    r: &Runner,
    seen: &Rc<RefCell<Result<bool, String>>>,
    clock: &Rc<Cell<i64>>,
) -> Vec<String> {
    let tick = || {
        let s = text(&r.tick(), "status").unwrap();
        clock.set(clock.get() + MIN);
        s
    };
    *seen.borrow_mut() = Ok(true);
    let mut said = vec![tick()];
    *seen.borrow_mut() = Ok(false);
    said.extend((0..16).map(|_| tick()));
    said
}

/// /code-review of ci-runner-pool: a heal skips the hour since the last reinstall, so two guards hold it. A heal whose
/// PATCH may have gone through (no answer) is counted, its runners no longer seen, so the next tick sends no second one;
/// and past one heal, the next waits the hour: a box that dies under every cold build is reinstalled once an hour, never
/// each 35 minutes (each reinstall a cold build again, and a Let's Encrypt order for whatsnr.eu, 5 a week).
#[dash_test]
fn a_heal_is_sent_once_and_heals_past_the_first_wait_the_hour() {
    let (inner, sent) = hostman(ON, 30.0, 0.0);
    let lost = Rc::new(Cell::new(true));
    let l = Rc::clone(&lost);
    let call: Tool = Box::new(move |asked: &str| {
        let answer = inner(asked);
        match asked.contains(r#""method":"PATCH""#) && l.get() {
            true => Err("hostman: no answer: timed out".into()),
            false => answer,
        }
    });
    let seen = Rc::new(RefCell::new(Ok(false)));
    let (r, clock) = clocked(call, want(), Rc::clone(&seen));
    let healed = died(&r, &seen, &clock);
    assert!(
        healed[16].starts_with("down: hostman: no answer"),
        "{healed:?}"
    );
    assert_eq!(sent.borrow().len(), 1);
    lost.set(false);
    let after: Vec<String> = (0..20)
        .map(|_| {
            clock.set(clock.get() + MIN);
            text(&r.tick(), "status").unwrap()
        })
        .collect();
    assert_eq!(sent.borrow().len(), 1, "no second PATCH: {after:?}");

    // heals, the PATCHes answered: the first within the hour, the next one waits the hour
    let (call, sent) = hostman(ON, 30.0, 0.0);
    let seen = Rc::new(RefCell::new(Ok(false)));
    let (r, clock) = clocked(call, want(), Rc::clone(&seen));
    let first = died(&r, &seen, &clock);
    assert!(
        first[16].contains("vanished after it was online"),
        "{first:?}"
    );
    let second = died(&r, &seen, &clock);
    assert!(
        second[16].starts_with("down: server 7: no runner online"),
        "{second:?}"
    );
    assert_eq!(sent.borrow().len(), 1, "one heal in the hour");
}

/// /code-review of ci-runner-pool: the whatsnr.eu redirect is the ci box's alone (one box holds its DNS), so ci-2 runs no
/// Caddy and opens no 80 or 443; every runner under ci also takes ci-light, so a light job runs on whichever box has a
/// free runner, and the light runner on ci takes light jobs alone, never a build.
#[dash_test]
fn only_the_ci_box_redirects_whatsnr_and_every_ci_runner_takes_light_jobs() {
    use super::live::pool;
    let [big, small] = &pool()[..] else {
        panic!("two boxes")
    };
    fn tokens(w: &Want) -> Vec<(&Reg, String)> {
        w.registrations
            .iter()
            .map(|r| (r, "T".to_string()))
            .collect()
    }
    let (b, s) = (
        cloud_init(big, &tokens(big), &[]),
        cloud_init(small, &tokens(small), &[]),
    );
    assert!(
        b.contains(", caddy]")
            && b.contains("/etc/caddy/Caddyfile")
            && b.contains("restart caddy docker")
    );
    assert!(!s.contains("caddy"), "{s}");
    assert!(s.contains("runcmd:\n  - systemctl restart docker\n"), "{s}");
    assert!(
        b.contains("--labels ci,ci-light --name ci --url https://github.com/nha-org/site_perso ")
    );
    assert!(b.contains("--labels ci-light --name ci-light --url"));
    assert!(s.contains("--labels ci,ci-light --name ci-2 --url"));
}

/// /code-review of ci-runner-pool: the boxes' runners share a registration's runner list, read once a tick, never once a
/// runner (three identical reads a minute for site_perso); the next tick reads it again.
#[dash_test]
fn a_runner_list_is_read_once_for_every_runner_of_its_registration() {
    use super::live::Lists;
    let reads = Cell::new(0);
    let lists = Lists::default();
    let read = || {
        reads.set(reads.get() + 1);
        Ok::<_, String>(format!("list {}", reads.get()))
    };
    assert_eq!(lists.get("repos/a", read), Ok("list 1".into()));
    assert_eq!(lists.get("repos/a", read), Ok("list 1".into()));
    assert_eq!(lists.get("repos/b", read), Ok("list 2".into()));
    lists.clear();
    assert_eq!(
        lists.get("repos/a", read),
        Ok("list 3".into()),
        "a new tick"
    );
    let failed = lists.get("repos/c", || Err("HTTP 502".into()));
    assert_eq!(failed, Err("HTTP 502".to_string()));
    assert_eq!(
        lists.get("repos/c", read),
        Ok("list 4".into()),
        "an error is never kept"
    );
}

/// The IPv4 is the hostman tool's to price, on the create (one price, one place): a create the preset alone fits and the
/// tool's cap refuses is held 15 minutes as a refusal here is, never a POST each minute.
#[dash_test]
fn a_create_the_tool_s_cap_refuses_is_held_as_one_refused_here() {
    let refusal = "hostman: POST /api/v1/servers refused: hostman: refused: a server on preset 5157 costs 22.00 USD a month, \
                   22.00 USD a month with the servers running, past the spend cap of 21 USD of credit left";
    let posts = Rc::new(Cell::new(0));
    let p = Rc::clone(&posts);
    let (inner, _) = hostman(NONE, 21.0, 0.0);
    let call: Tool = Box::new(
        move |asked: &str| match asked.contains(r#""method":"POST""#) {
            true => {
                p.set(p.get() + 1);
                Err(refusal.into())
            }
            false => inner(asked),
        },
    );
    let clock = Rc::new(Cell::new(0));
    let now = Rc::clone(&clock);
    let r = runner_with(call, |_| Ok(Seen::Gone)).clocked(Box::new(move || now.get()));
    let alarms: Vec<String> = (0..10)
        .filter_map(|_| {
            clock.set(clock.get() + MIN);
            text(&r.tick(), "alarm")
        })
        .collect();
    assert_eq!(alarms, [refusal]);
    assert_eq!(posts.get(), 1, "one POST in ten minutes");
}

/// One box carries the light runner: its name is its label (`Reg::runner`), so a second would replace the first on GitHub.
#[dash_test]
fn exactly_one_box_of_the_pool_carries_the_light_runner() {
    let light = |w: &Want| w.registrations.iter().any(|r| r.label == LIGHT);
    assert_eq!(super::live::pool().iter().filter(|w| light(w)).count(), 1);
}

/// Each tick records Hostman's balance in this plugin's own rows, read only through the hostman tool, so `dash infra
/// plan`'s guard holds a growing month against it (rfc-iac S4). An answer with no balance writes nothing.
#[dash_test]
fn the_balance_is_recorded_from_the_finances_answer() {
    use super::live::credit_row;
    let answer = r#"{"finances":{"balance":27.16,"currency":"USD","monthly_cost":22}}"#;
    let row = credit_row(answer).expect("a balance");
    assert_eq!(
        (row.table.as_str(), row.row.as_str(), row.field.as_str()),
        ("ci_credit", "hostman", "balance")
    );
    assert_eq!(row.value, Value::Text("27.16".into()));
    assert!(credit_row(r#"{"finances":{}}"#).is_none());
    assert!(credit_row("not json").is_none());
}
