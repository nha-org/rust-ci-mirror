//! Rust's Datadog monitors reach him (task rust-monitor-tells-him; @nha, 2026-11-06: « so we don't grow accidentally. »):
//! `dash orch` asks Datadog every minute (`watchers::ALERTS`) for the monitors tagged `service:dash-rust` (rfc-rust-unified-datadog.exs saves
//! them) and pushes each group newly in Alert to his phone, once, through the old app's `dash push`. Datadog cannot call
//! this Mac, so the Mac asks. `dash alerts --data <dir>` runs one pass.
//!
//! A group on a path no Rust page registers (`pages::find`, the inventory) is dropped before any push and named in the pass's
//! line, and so is one on `route:route_older`, the forward to the old app (task rust-alerts-old-route): the old app's pages are
//! not his to hear of (task rust-alerts-rust-pages; @nha, 2026-11-11: « /v/private/docs/rfc-inbox.html served by elixir so I
//! don't care about DD alert 5s »).
//!
//! ponytail: the old app's `dash push`, the one push a command can send today: urgent, so it rings in the quiet hours too, and
//! listed on /help. Rust's phone crate replaces it when Dashboard.Phone is ported (rfc-rust-unified section 20d).
//!
//! Off with the telemetry switch (`dash telemetry off`): no switch, no question to Datadog, so tests and CI ask nothing. The keys
//! come from bin/secret (datadog: DD_API_KEY, RUST_DD_API_KEY), into curl's environment, never its command line.
use crate::tasks::{now_ms, older_app, repo};
use crate::watchers;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

const ASK: Duration = Duration::from_secs(20);
/// Past bin/dash's own 60 s and its retries: a slow push that arrived must not count as failed, or it rings again next pass.
const PUSH: Duration = Duration::from_secs(150);
/// The `route` tag of an answer the old app owes: the front door's 502 when it does not answer (main.rs). His words on it:
/// « served by elixir so I don't care » (@nha, 2026-11-11).
pub const OLDER_ROUTE: &str = "route_older";
const MONITORS: &str = "https://api.datadoghq.eu/api/v1/monitor?monitor_tags=service:dash-rust&group_states=alert&with_downtimes=true";

/// A group in Alert: its monitor's id and name, and the group (`command:serve`, or `*` for a monitor with none).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Alerting {
    pub id: i64,
    pub name: String,
    pub group: String,
}

impl Alerting {
    /// Whether this group is one Rust answers: not on `route:route_older`, the forward to the old app (task rust-alerts-old-route),
    /// and on a page the inventory holds when it names one; a group with neither tag (a process's) always is.
    fn on_a_rust_page(&self) -> bool {
        let tag = |name: &str| self.group.split(',').find_map(|t| t.strip_prefix(name));
        tag("route:") != Some(OLDER_ROUTE)
            && tag("resource_name:").is_none_or(|p| crate::pages::find(p).is_some())
    }

    /// What his phone shows.
    fn words(&self) -> String {
        format!(
            "Datadog: « {} » is alerting on {}. https://app.datadoghq.eu/monitors/{}",
            self.name, self.group, self.id
        )
    }
}

/// The groups in Alert in Datadog's answer to `MONITORS`; a monitor in Alert that names no group is one group, `*`. A monitor
/// he muted, or under a downtime, says nothing: `dash push` is urgent and rings through the quiet hours.
pub fn alerting(answer: &str) -> Result<BTreeSet<Alerting>, String> {
    let monitors: Vec<Value> =
        serde_json::from_str(answer).map_err(|e| format!("Datadog's monitors: {e}"))?;
    let mut all = BTreeSet::new();
    for m in &monitors {
        let (Some(id), Some(name)) = (m["id"].as_i64(), m["name"].as_str()) else {
            continue;
        };
        let muted = m.pointer("/options/silenced").and_then(Value::as_object);
        let down = m["matching_downtimes"].as_array();
        if muted.is_some_and(|s| !s.is_empty()) || down.is_some_and(|d| !d.is_empty()) {
            continue;
        }
        let at = |group: &str| Alerting {
            id,
            name: name.into(),
            group: group.into(),
        };
        let groups = m.pointer("/state/groups").and_then(Value::as_object);
        let named: Vec<&String> = groups
            .into_iter()
            .flatten()
            .filter(|(_, g)| g["status"] == "Alert")
            .map(|(group, _)| group)
            .collect();
        all.extend(named.iter().map(|g| at(g)));
        if groups.is_none_or(|g| g.is_empty()) && m["overall_state"] == "Alert" {
            all.insert(at("*"));
        }
    }
    Ok(all)
}

/// The groups he was told of and that still alerted at the last pass: (monitor id, group).
fn told(dir: &Path) -> Result<BTreeSet<(i64, String)>, String> {
    let db = watchers::open(dir)?;
    let mut read = db
        .prepare("SELECT id, grp FROM told")
        .map_err(|e| e.to_string())?;
    let rows = read
        .query_map((), |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}

fn keep(dir: &Path, told: &BTreeSet<(i64, String)>) -> Result<(), String> {
    let mut db = watchers::open(dir)?;
    let tx = db.transaction().map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM told", ())
        .map_err(|e| e.to_string())?;
    for (id, group) in told {
        tx.execute("INSERT INTO told (id, grp) VALUES (?1, ?2)", (id, group))
            .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

/// One pass: `ask` Datadog, `tell` him of each group newly in Alert, keep what he was told. A group that recovers is
/// forgotten, so its next breach is told again; one whose push failed is not kept, so the next pass tries it again.
pub fn pass(
    dir: &Path,
    ask: &dyn Fn() -> Result<String, String>,
    tell: &dyn Fn(&str) -> Result<(), String>,
) -> Result<String, String> {
    if dash_statsd::endpoint(dir).is_none() {
        return Ok("telemetry off: Datadog not asked".into());
    }
    let (now, dropped): (BTreeSet<_>, BTreeSet<_>) = alerting(&ask()?)?
        .into_iter()
        .partition(Alerting::on_a_rust_page);
    let before = told(dir)?;
    let key = |a: &Alerting| (a.id, a.group.clone());
    let mut kept: BTreeSet<_> = now.iter().map(key).filter(|k| before.contains(k)).collect();
    let (mut said, mut failed) = (vec![], vec![]);
    for a in now.iter().filter(|a| !before.contains(&key(a))) {
        match tell(&a.words()) {
            Ok(()) => {
                kept.insert(key(a));
                said.push(format!("{} on {}", a.id, a.group));
            }
            Err(e) => failed.push(format!("{} on {}: {e}", a.id, a.group)),
        }
    }
    keep(dir, &kept)?;
    if !said.is_empty() {
        let what = format!("him, by push: {}", said.join(", "));
        if let Err(e) = watchers::woke(dir, &watchers::ALERTS, now_ms(), &what) {
            eprintln!("alerts: the wake's record: {e}");
        }
    }
    let line = format!(
        "{} alerting, told {}{}{}",
        now.len(),
        if said.is_empty() {
            "none".into()
        } else {
            said.join(", ")
        },
        if failed.is_empty() {
            String::new()
        } else {
            format!("; not told: {}", failed.join("; "))
        },
        if dropped.is_empty() {
            String::new()
        } else {
            let each: Vec<_> = dropped
                .iter()
                .map(|a| format!("{} on {}", a.id, a.group))
                .collect();
            format!("; dropped, not Rust: {}", each.join(", "))
        }
    );
    if failed.is_empty() {
        Ok(line)
    } else {
        Err(line)
    }
}

fn secret(name: &str) -> Result<String, String> {
    let bin = repo().join("bin/secret").display().to_string();
    let key = dash_process::run(&bin, &["get", "datadog", name], None, ASK)?;
    Ok(key.trim().to_string())
}

/// Both keys, read once a process: bin/secret is a Python start each. A read that fails is tried again next pass.
/// ponytail: a key changed in bin/secret reaches `dash orch` at its restart.
fn keys() -> Result<&'static (String, String), String> {
    static KEYS: OnceLock<(String, String)> = OnceLock::new();
    if let Some(k) = KEYS.get() {
        return Ok(k);
    }
    let read = (secret("DD_API_KEY")?, secret("RUST_DD_API_KEY")?);
    Ok(KEYS.get_or_init(|| read))
}

/// Datadog's monitors of dash-rust.
fn ask() -> Result<String, String> {
    get(MONITORS, &[])
}

/// A read of Datadog's API over curl, the keys in its environment, `query`'s pairs encoded by curl (`dash fallback`'s share).
pub(crate) fn get(url: &str, query: &[(&str, &str)]) -> Result<String, String> {
    let (api, app) = keys()?;
    let mut args: Vec<String> = [
        "-sS",
        "--max-time",
        "15",
        "--variable",
        "%API",
        "--variable",
        "%APP",
        "--expand-header",
        "DD-API-KEY: {{API}}",
        "--expand-header",
        "DD-APPLICATION-KEY: {{APP}}",
        "-w",
        "\n%{http_code}",
        url,
    ]
    .map(String::from)
    .into();
    for (name, value) in query {
        args.extend([
            "-G".into(),
            "--data-urlencode".into(),
            format!("{name}={value}"),
        ]);
    }
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let env = [("API", api.as_str()), ("APP", app.as_str())];
    let (status, text, reason) =
        dash_process::answer("curl", &args, None, &env, ASK).map_err(String::from)?;
    let (body, code) = text
        .trim_end()
        .rsplit_once('\n')
        .unwrap_or(("", text.trim()));
    match code {
        _ if !status.success() => Err(format!("Datadog: no answer: {}", reason.trim())),
        "200" => Ok(body.to_string()),
        code => Err(format!(
            "Datadog answered {code}: {}",
            body.chars().take(200).collect::<String>()
        )),
    }
}

/// His phone, through the old app's `dash push`.
pub(crate) fn tell(words: &str) -> Result<(), String> {
    dash_process::run(&older_app().bin, &["push", words], None, PUSH).map(drop)
}

/// A pass and its beat on /watchers, whatever it said.
fn beating(dir: &Path) -> Result<String, String> {
    let ran = pass(dir, &ask, &tell);
    let said = ran
        .as_ref()
        .map_or_else(|e| format!("alerts: {e}"), String::clone);
    if let Err(e) = watchers::beat(dir, &watchers::ALERTS, now_ms(), ran.is_ok(), &said) {
        eprintln!("alerts: the beat: {e}");
    }
    ran
}

inventory::submit! { crate::usage::Command::usage("dash alerts --data <dir>") }
pub fn once(dir: &Path, mut out: File) {
    match beating(dir) {
        Ok(line) => writeln!(out, "{line}").expect("stdout"),
        Err(e) => {
            eprintln!("alerts: {e}");
            std::process::exit(1);
        }
    }
}

/// A pass every `watchers::ALERTS.every_ms` for as long as `dash orch` runs; a line on stderr each time what it says changes.
pub fn watch(dir: PathBuf) {
    let sleep = Duration::from_millis(watchers::ALERTS.every_ms as u64);
    watchers::run(dir, &watchers::ALERTS, sleep, beating);
}

/// `dash warm times [<hours>]`: each page's time to its warmed content after each start of `dash serve` in the last hours,
/// read back from Datadog (`dash_rust.page.warm_ms`), so a restart's numbers are a command, never a hand-written query.
pub fn times(hours: u64, mut out: std::fs::File) {
    use std::io::Write;
    let to = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let from = to.saturating_sub(hours as i64 * 3600);
    let source = warm_source(from, to);
    let (from, to) = (from.to_string(), to.to_string());
    let pairs = [
        ("from", from.as_str()),
        ("to", to.as_str()),
        ("query", WARM_QUERY),
    ];
    let said =
        get("https://api.datadoghq.eu/api/v1/query", &pairs).and_then(|body| warm_lines(&body));
    match said {
        Ok(lines) => write!(out, "{source}{lines}").expect("stdout"),
        Err(e) => {
            eprintln!("warm times: {e}");
            std::process::exit(1);
        }
    }
}

/// The warm times' query: one sample a page and start, so `fill(null)` keeps Datadog from drawing points between them (two
/// a start, 2 s apart, on the first live read).
const WARM_QUERY: &str = "max:dash_rust.page.warm_ms.max{service:dash-rust} by {page}.fill(null)";

/// Where the lines come from, printed above them: the query as Datadog's API was asked it, and the same metric in its
/// explorer over the same hours, so a reader sees Datadog's numbers, never only ours (the PM check of rust-warmup-queue
/// could not tell them apart).
fn warm_source(from: i64, to: i64) -> String {
    format!(
        "Datadog: GET https://api.datadoghq.eu/api/v1/query?query={WARM_QUERY}&from={from}&to={to}\n\
         Datadog explorer: https://app.datadoghq.eu/metric/explorer?from_ts={}&to_ts={}&live=false\
         &exp_metric=dash_rust.page.warm_ms.max&exp_scope=service%3Adash-rust&exp_agg=max&exp_row_type=metric\
         &exp_group=page\n",
        from * 1000,
        to * 1000
    )
}

/// Datadog's answer as one line a point: its time (UTC), its page, its ms; « none » when no start sent one.
fn warm_lines(body: &str) -> Result<String, String> {
    let answer: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("{e}: {body}"))?;
    if let Some(errors) = answer.get("errors") {
        return Err(errors.to_string());
    }
    let mut lines = vec![];
    for series in answer["series"].as_array().into_iter().flatten() {
        // its scope is every tag it is grouped and filtered by: « page:/tools,service:dash-rust »
        let scope = series["scope"].as_str().unwrap_or("");
        let page = scope
            .split(',')
            .find_map(|t| t.strip_prefix("page:"))
            .unwrap_or(scope);
        for point in series["pointlist"].as_array().into_iter().flatten() {
            if let (Some(at), Some(ms)) = (point[0].as_f64(), point[1].as_f64()) {
                let s = at as i64 / 1000;
                let date = dash_core::date(s.div_euclid(86_400));
                let (h, m) = (s.rem_euclid(86_400) / 3600, s.rem_euclid(3600) / 60);
                lines.push(format!("{date}T{h:02}:{m:02}Z {page} {ms:.0} ms"));
            }
        }
    }
    lines.sort();
    Ok(match lines.is_empty() {
        true => "none: no start sent a page's warm time\n".into(),
        false => lines.join("\n") + "\n",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::memory::TestBudget;
    use std::cell::RefCell;

    /// Datadog's answer, cut: the memory monitor with serve at `serve`, orch OK; one alerting with no group.
    fn answer(serve: &str) -> String {
        serde_json::json!([
            {"id": 130136039, "name": "dash-rust: memory past the idle bar of 100 MB", "overall_state": serve,
             "state": {"groups": {"command:serve": {"status": serve}, "command:orch": {"status": "OK"}}}},
            {"id": 7, "name": "dash-rust: an answer of 400 and over", "overall_state": "OK", "state": {"groups": {}}},
            {"id": 8, "name": "no groups", "overall_state": "Alert"},
            {"id": 9, "name": "muted", "overall_state": "Alert", "options": {"silenced": {"*": null}}},
            {"id": 10, "name": "in a downtime", "overall_state": "Alert", "matching_downtimes": [{"id": 1}]}
        ])
        .to_string()
    }

    /// Task rust-warmup-queue, done-when 1: `dash warm times` reads each page's warm time back from Datadog, one line a point.
    #[test]
    fn warm_times_are_a_line_per_page_and_start() {
        let _budget = TestBudget::new();
        let body = r#"{"series":[{"scope":"page:/tools,service:dash-rust","pointlist":[[1762900020000.0,8120.5]]},
            {"scope":"page:/insights","pointlist":[[1762900020000.0,31800.0],[1762900140000.0,null]]}]}"#;
        assert_eq!(
            warm_lines(body),
            Ok("2025-11-11T22:27Z /insights 31800 ms\n2025-11-11T22:27Z /tools 8120 ms\n".into())
        );
        assert_eq!(
            warm_lines(r#"{"series":[]}"#),
            Ok("none: no start sent a page's warm time\n".into())
        );
        assert!(warm_lines(r#"{"errors":["bad query"]}"#).is_err());
        let source = warm_source(1_762_900_000, 1_762_907_200);
        assert!(source.starts_with("Datadog: GET https://api.datadoghq.eu/api/v1/query?query=max:dash_rust.page.warm_ms.max"));
        assert!(
            source.contains("from=1762900000&to=1762907200\n"),
            "{source}"
        );
        assert!(
            source.contains("explorer?from_ts=1762900000000&to_ts=1762907200000&"),
            "{source}"
        );
    }

    #[test]
    fn a_breach_is_told_once_and_again_after_it_recovers() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("alerts");
        let pushed = RefCell::new(vec![]);
        let tell = |w: &str| {
            pushed.borrow_mut().push(w.to_string());
            Ok(())
        };
        let now = RefCell::new(answer("Alert"));
        let ask = || Ok(now.borrow().clone());

        // off: Datadog is not asked
        assert_eq!(
            pass(&dir.0, &ask, &tell).unwrap(),
            "telemetry off: Datadog not asked"
        );
        dash_statsd::set(&dir.0, true).expect("on");

        assert_eq!(
            pass(&dir.0, &ask, &tell).unwrap(),
            "2 alerting, told 8 on *, 130136039 on command:serve"
        );
        assert_eq!(
            pushed.borrow()[1],
            "Datadog: « dash-rust: memory past the idle bar of 100 MB » is alerting on command:serve. \
             https://app.datadoghq.eu/monitors/130136039"
        );
        // still alerting: nothing more
        assert_eq!(pass(&dir.0, &ask, &tell).unwrap(), "2 alerting, told none");
        assert_eq!(pushed.borrow().len(), 2);
        // recovered, then past the bar again: told again
        *now.borrow_mut() = answer("OK");
        pass(&dir.0, &ask, &tell).unwrap();
        *now.borrow_mut() = answer("Alert");
        assert_eq!(
            pass(&dir.0, &ask, &tell).unwrap(),
            "2 alerting, told 130136039 on command:serve"
        );
        assert_eq!(pushed.borrow().len(), 3);
    }

    /// Task rust-alerts-rust-pages, done-when 1 and 2 (« Inventory decides », the task dialog's answer): the slow-page
    /// monitor's groups on paths no Rust page registers, /v/ and /projects/, are dropped before any push and named in the
    /// line; /drafts/<id>, a Rust page (`/drafts/*`), still pushes, and so does a group with no path.
    #[test]
    fn only_an_alert_on_a_rust_page_reaches_him() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("alerts-rust-pages");
        dash_statsd::set(&dir.0, true).expect("on");
        let slow = serde_json::json!([
            {"id": 130409982, "name": "dash-rust: a page past 5 s", "overall_state": "Alert", "state": {"groups": {
                "resource_name:/v/private/docs/rfc-inbox.html": {"status": "Alert"},
                "resource_name:/projects/site_perso/issue/rust-usage-registered": {"status": "Alert"},
                "resource_name:/drafts/2026-11-10-hostman": {"status": "Alert"}}}},
            {"id": 8, "name": "no groups", "overall_state": "Alert"}
        ])
        .to_string();
        let pushed = RefCell::new(vec![]);
        let tell = |w: &str| {
            pushed.borrow_mut().push(w.to_string());
            Ok(())
        };
        let said = pass(&dir.0, &|| Ok(slow.clone()), &tell).unwrap();
        assert_eq!(
            said,
            "2 alerting, told 8 on *, 130409982 on resource_name:/drafts/2026-11-10-hostman; dropped, not Rust: \
             130409982 on resource_name:/projects/site_perso/issue/rust-usage-registered, \
             130409982 on resource_name:/v/private/docs/rfc-inbox.html"
        );
        assert_eq!(pushed.borrow().len(), 2, "{:?}", pushed.borrow());
        assert!(
            pushed
                .borrow()
                .iter()
                .all(|w| !w.contains("/v/") && !w.contains("/projects/"))
        );
        // dropped, never kept: the next pass pushes nothing and names them again
        let again = pass(&dir.0, &|| Ok(slow.clone()), &tell).unwrap();
        assert!(
            again.starts_with("2 alerting, told none; dropped, not Rust: "),
            "{again}"
        );
        assert_eq!(pushed.borrow().len(), 2);
    }

    /// The 400+ monitor in Alert on `group`: the lines of two passes, and what was pushed.
    fn refused_on(group: &str) -> (String, String, Vec<String>) {
        let dir = TestDir::new("alerts-routes");
        dash_statsd::set(&dir.0, true).expect("on");
        let refused = serde_json::json!([
            {"id": 7, "name": "dash-rust: an answer of 400 and over", "overall_state": "Alert",
             "state": {"groups": {group: {"status": "Alert"}}}}
        ])
        .to_string();
        let pushed = RefCell::new(vec![]);
        let tell = |w: &str| {
            pushed.borrow_mut().push(w.to_string());
            Ok(())
        };
        let line = pass(&dir.0, &|| Ok(refused.clone()), &tell).unwrap();
        let again = pass(&dir.0, &|| Ok(refused.clone()), &tell).unwrap();
        (line, again, pushed.into_inner())
    }

    /// Task rust-alerts-old-route, done-when 1: the front door's 502 on the forward to the old app is dropped and named.
    #[test]
    fn an_old_app_route_alert_is_dropped() {
        let _budget = TestBudget::new();
        let (line, again, pushed) = refused_on("route:route_older,status:502");
        let dropped = "0 alerting, told none; dropped, not Rust: 7 on route:route_older,status:502";
        assert_eq!(line, dropped);
        // dropped, never kept: the next pass names it again and still pushes nothing
        assert_eq!(again, dropped);
        assert!(pushed.is_empty(), "{pushed:?}");
    }

    /// Task rust-alerts-old-route, done-when 2: a 502 on a route Rust answers still rings.
    #[test]
    fn a_rust_route_alert_still_pushes() {
        let _budget = TestBudget::new();
        let (line, again, pushed) = refused_on("route:route_draft,status:502");
        assert_eq!(line, "1 alerting, told 7 on route:route_draft,status:502");
        assert_eq!(again, "1 alerting, told none");
        assert_eq!(pushed.len(), 1, "{pushed:?}");
    }

    /// Datadog lowercases a `resource_name` tag and turns any other character but `_-:./` into `_`: a page path it would change
    /// never matches its alert's group, so that page's alerts would be dropped. None may hold one (/code-review).
    #[test]
    fn every_page_path_survives_datadog_s_tag_normalizing() {
        let _budget = TestBudget::new();
        for page in inventory::iter::<crate::pages::Page> {
            let normal =
                |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || "/-_.*".contains(c);
            assert!(
                page.path.chars().all(normal),
                "{}: Datadog rewrites it",
                page.path
            );
        }
    }

    #[test]
    fn a_push_that_fails_is_tried_again_and_a_failed_ask_tells_nothing() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("alerts-fail");
        dash_statsd::set(&dir.0, true).expect("on");
        let ask = || Ok(answer("Alert"));
        let down = |_: &str| Err("the old app is down".to_string());
        let said = pass(&dir.0, &ask, &down).unwrap_err();
        assert!(
            said.starts_with("2 alerting, told none; not told: 8 on *: the old app is down"),
            "{said}"
        );
        let pushed = RefCell::new(0);
        let tell = |_: &str| {
            *pushed.borrow_mut() += 1;
            Ok(())
        };
        pass(&dir.0, &ask, &tell).unwrap();
        assert_eq!(*pushed.borrow(), 2);
        let refused = || Err("Datadog answered 403".to_string());
        assert_eq!(
            pass(&dir.0, &refused, &tell).unwrap_err(),
            "Datadog answered 403"
        );
        assert_eq!(*pushed.borrow(), 2);
    }
}
