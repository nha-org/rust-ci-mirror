//! Rust's Datadog monitors, service dash-rust, held here as data and saved by `dash monitors save` (task rust-monitors-port;
//! @nha, 2026-11-10: « no diffs on ex or exs at all, rust HAS to take over »). They replace rfc-rust-unified-datadog.exs,
//! deleted whole. `dash monitors state` prints each and its last day of transitions; `dash monitors read [<seconds>]` prints
//! rfc-rust-unified's baseline. `dash orch` hears of their alerts through `alerts.rs`, by the tag `service:dash-rust`.
//!
//! Each monitor is saved in place by its id, a PUT: never a POST, so a run can never leave a duplicate. A monitor deleted at
//! Datadog is an error naming its id, to be created by hand once and its id written here.
use crate::alerts::{call, get};
use serde_json::{Value, json};
use std::fs::File;
use std::io::Write;

const API: &str = "https://api.datadoghq.eu/api/v1";

/// One monitor as Datadog keeps it, its id the one it was first saved under.
struct Monitor {
    id: i64,
    name: &'static str,
    query: &'static str,
    message: &'static str,
    /// The bar the query ends with.
    critical: u32,
    /// Whether Datadog waits for a whole window of points: a gauge read every 30 s has one, a sparse count does not.
    full_window: bool,
}

/// The three monitors of dash-rust.
const MONITORS: [Monitor; 3] = [
    // The memory a process holds now, past the idle bar for 15 minutes on end: `min` over the window, so one reading under the
    // bar ends it. Never the lifetime peak_mb: it stays at its high for as long as the process runs, and the monitor was red
    // 2026-11-12 02:40 while serve's current memory had fallen to 66 MB (task rust-monitors-port, done-when 3). The bar is
    // crates/dash/tests/memory_budgets.rs's. A process that ended says no more: resolved after timeout_h.
    Monitor {
        id: 130136039,
        name: "dash-rust: memory past the idle bar of 100 MB",
        query: "min(last_15m):max:dash_rust.memory.current_mb{service:dash-rust} by {command} > 100",
        message: "A Rust process held more than 100 MB for 15 minutes, rfc-rust-unified's idle bar. Which one: the command tag. \
                  Now {{value}} MB at its lowest in the window, against the bars in rfc-rust-unified section 20c.",
        critical: 100,
        full_window: true,
    },
    // One refused answer is worth a look (@nha, 2026-11-06: « had one "bad request" already fyi. we need to monitor »), so the
    // bar is above zero. Sparse by nature: default_zero resolves it after 5 quiet minutes, so each incident is its own transition.
    Monitor {
        id: 130163768,
        name: "dash-rust: an answer of 400 and over",
        query: "sum(last_5m):default_zero(sum:dash_rust.http.refused{service:dash-rust} by {status,route}.as_count()) > 0",
        message: "dash serve refused {{value}} requests in 5 minutes. Which: the status and route tags. Why: its log, \
                  `answered <status> on <route>`; status:closed got no answer, its lines are `past 64 connections`, \
                  `no thread for a connection` and `no connection accepted`.",
        critical: 0,
        full_window: false,
    },
    // A page slow to answer tells him (@nha, 2026-11-10: « show 60 to 138 s in Datadog => there should be some alert + watcher
    // then I should not have to point this out »), but only a slowness that lasts: three requests past 5 s in 15 minutes, a
    // count Datadog takes of the distribution itself (`count(v: v>5)`). The p95 of 10 minutes it replaces rang on one slow
    // request each time, 20 times on 2026-11-10 and 11 (task rust-monitors-port, done-when 2). Left out, as they stay open by
    // design: LiveView's websocket and longpoll, the CLI's calls (a ship runs minutes), and the board's patch stream,
    // /tasks?view=patches (Datadog writes it /tasks_view_val).
    Monitor {
        id: 130409982,
        name: "dash-rust: a page past 5 s",
        query: "sum(last_15m):count(v: v>5):trace.http.request{service:dash-rust,!resource_name:/dashboard/live/websocket,\
                !resource_name:/dashboard/live/longpoll,!resource_name:/dashboard/cli,!resource_name:/tasks_view_val} \
                by {resource_name}.as_count() > 2",
        message: "{{resource_name.name}}: {{value}} requests past 5 s in 15 minutes. Why: their traces' child spans in APM, \
                  service dash-rust; a gap between spans is a read no span covers yet.",
        critical: 2,
        full_window: false,
    },
];

impl Monitor {
    /// The body Datadog keeps: the tag `service:dash-rust` is how `alerts.rs` finds it. A process that ended says no more:
    /// its alert resolves after `timeout_h`.
    fn body(&self) -> Value {
        let options = json!({"thresholds": {"critical": self.critical}, "notify_no_data": false,
                             "require_full_window": self.full_window, "timeout_h": 1, "include_tags": true});
        json!({"type": "metric alert", "name": self.name, "query": self.query, "message": self.message,
               "tags": ["service:dash-rust"], "options": options})
    }
}

/// One saved or read monitor as two lines: its id, state and query, then its page.
fn shown(m: &Value) -> String {
    format!(
        "monitor {}: {}, {}\nhttps://app.datadoghq.eu/monitors/{}\n",
        m["id"],
        m["overall_state"].as_str().unwrap_or("?"),
        m["query"].as_str().unwrap_or("?"),
        m["id"]
    )
}

fn parsed(body: &str) -> Result<Value, String> {
    serde_json::from_str(body).map_err(|e| format!("Datadog's answer: {e}"))
}

/// Each monitor PUT in place; one Datadog refuses does not stop the others, and each refusal is named.
fn save(out: &mut File) -> Result<(), String> {
    let mut refused = vec![];
    for m in &MONITORS {
        let url = format!("{API}/monitor/{}", m.id);
        match call("PUT", &url, &[], Some(&m.body().to_string())).and_then(|a| parsed(&a)) {
            Ok(saved) => write!(out, "{}", shown(&saved)).map_err(|e| e.to_string())?,
            Err(e) => refused.push(format!("monitor {}: {e}", m.id)),
        }
    }
    if refused.is_empty() {
        Ok(())
    } else {
        Err(refused.join("; "))
    }
}

/// The transitions of the last day in Datadog's events answer, one line each, oldest first.
fn transitions(id: i64, answer: &Value) -> Vec<String> {
    let mut events: Vec<&Value> = answer["events"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| e["monitor_id"].as_i64() == Some(id))
        .collect();
    events.sort_by_key(|e| e["date_happened"].as_i64());
    events
        .iter()
        .map(|e| {
            format!(
                "{} {}: {}",
                dash_core::iso(e["date_happened"].as_i64().unwrap_or(0)),
                e["alert_type"].as_str().unwrap_or("?"),
                e["title"].as_str().unwrap_or("?")
            )
        })
        .collect()
}

fn state(out: &mut File) -> Result<(), String> {
    let now = crate::tasks::now_ms() / 1000;
    for m in &MONITORS {
        let read = parsed(&get(
            &format!("{API}/monitor/{}", m.id),
            &[("group_states", "all")],
        )?)?;
        let (start, end, text) = (
            (now - 86_400).to_string(),
            now.to_string(),
            format!("monitor {}", m.id),
        );
        let pairs = [
            ("start", start.as_str()),
            ("end", end.as_str()),
            ("sources", "alert"),
            ("tags", "monitor"),
            ("unaggregated", "true"),
            ("text", text.as_str()),
        ];
        let events = parsed(&get(&format!("{API}/events"), &pairs)?)?;
        let mut said = shown(&read);
        for line in transitions(m.id, &events) {
            said += &line;
            said.push('\n');
        }
        write!(out, "{said}").map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// rfc-rust-unified's baseline over the last `window` seconds: each metric by command, its median and max.
fn read(window: i64, out: &mut File) -> Result<(), String> {
    let now = crate::tasks::now_ms() / 1000;
    let (from, to) = ((now - window).to_string(), now.to_string());
    for (label, metric) in [
        ("memory now, MB", "max:dash_rust.memory.current_mb"),
        ("memory peak, MB", "max:dash_rust.memory.peak_mb.max"),
        (
            "heap the cap counts now, MB",
            "max:dash_rust.memory.heap_mb",
        ),
        (
            "heap the cap counts, peak, MB",
            "max:dash_rust.memory.heap_peak_mb.max",
        ),
        ("start, ms", "max:dash_rust.start_ms.max"),
        (
            "step, ms: median of the p50s, max",
            "avg:dash_rust.step_ms.median",
        ),
    ] {
        let group = if label.contains("step") {
            "command,step"
        } else {
            "command"
        };
        let query = format!("{metric}{{service:dash-rust}} by {{{group}}}");
        let pairs = [
            ("from", from.as_str()),
            ("to", to.as_str()),
            ("query", query.as_str()),
        ];
        let answer = parsed(&get(&format!("{API}/query"), &pairs)?)?;
        let mut said = format!("{label} ({metric}, last {window} s)\n");
        said += &baseline(&answer);
        write!(out, "{said}").map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A query's answer as one line a series, by scope: the median and max of its points, and their count.
fn baseline(answer: &Value) -> String {
    let mut series: Vec<&Value> = answer["series"].as_array().into_iter().flatten().collect();
    series.sort_by_key(|s| s["scope"].as_str().unwrap_or("").to_string());
    let mut said = String::new();
    for s in series {
        let mut points: Vec<f64> = s["pointlist"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p[1].as_f64())
            .collect();
        if points.is_empty() {
            continue;
        }
        points.sort_by(f64::total_cmp);
        let (median, max) = (points[points.len() / 2], points[points.len() - 1]);
        said += &format!(
            "  {}: median {median:.1}, max {max:.1}, {} points\n",
            s["scope"].as_str().unwrap_or("?"),
            points.len()
        );
    }
    said
}

inventory::submit! { crate::usage::Command::usage("dash monitors save|state|read [<seconds>]") }
/// `dash monitors <what>`: exit 1 with Datadog's refusal on stderr.
pub fn command(what: &[&str], mut out: File) {
    let ran = match what {
        ["save"] => save(&mut out),
        ["state"] => state(&mut out),
        ["read"] => read(3600, &mut out),
        ["read", seconds] => match seconds.parse() {
            Ok(window) if window > 0 => read(window, &mut out),
            _ => Err(format!(
                "read: {seconds} is not a number of seconds above 0"
            )),
        },
        _ => Err("usage: dash monitors save|state|read [<seconds>]".into()),
    };
    if let Err(e) = ran {
        eprintln!("monitors: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// Done-when 1: the three monitors as data, each saved in place under the id Datadog gave it, each tagged so `alerts.rs`
    /// finds it.
    #[dash_test]
    fn the_three_monitors_are_data_saved_in_place_by_id() {
        let ids: Vec<i64> = MONITORS.iter().map(|m| m.id).collect();
        assert_eq!(ids, [130136039, 130163768, 130409982]);
        for m in &MONITORS {
            let body = m.body();
            assert_eq!(body["tags"], json!(["service:dash-rust"]), "{}", m.name);
            assert_eq!(body["type"], "metric alert");
            let bar = format!("> {}", body["options"]["thresholds"]["critical"]);
            assert!(
                m.query.ends_with(&bar),
                "{}: its query's bar {bar} is its threshold",
                m.name
            );
            assert_eq!(
                body["options"]["timeout_h"], 1,
                "{}: an ended process's alert resolves",
                m.name
            );
            assert!(
                !m.query.contains("  "),
                "{}: a line break left spaces in its query",
                m.name
            );
        }
        let saved =
            shown(&json!({"id": 130409982, "overall_state": "OK", "query": MONITORS[2].query}));
        assert!(
            saved.starts_with(
                "monitor 130409982: OK, sum(last_15m):count(v: v>5):trace.http.request{"
            )
        );
        assert!(saved.ends_with("\nhttps://app.datadoghq.eu/monitors/130409982\n"));
    }

    /// Done-when 2: the page monitor counts the requests past 5 s in 15 minutes and alerts on three, so one slow request never
    /// rings; still by page, still without the streams left open by design.
    #[dash_test]
    fn the_page_monitor_needs_several_slow_requests() {
        let page = &MONITORS[2];
        assert!(
            page.query
                .starts_with("sum(last_15m):count(v: v>5):trace.http.request{service:dash-rust,")
        );
        assert!(
            page.query.ends_with(" by {resource_name}.as_count() > 2"),
            "{}",
            page.query
        );
        assert!(
            !page.query.contains("p95"),
            "a percentile of one request is that request"
        );
        for left in [
            "/dashboard/live/websocket",
            "/dashboard/live/longpoll",
            "/dashboard/cli",
            "/tasks_view_val",
        ] {
            assert!(
                page.query.contains(&format!("!resource_name:{left}")),
                "{left}"
            );
        }
    }

    /// Done-when 3: the memory monitor reads what a process holds now, over 15 minutes, never its lifetime peak.
    #[dash_test]
    fn the_memory_monitor_reads_current_mb_over_15_minutes() {
        let memory = &MONITORS[0];
        assert_eq!(
            memory.query,
            "min(last_15m):max:dash_rust.memory.current_mb{service:dash-rust} by {command} > 100"
        );
        assert!(!memory.query.contains("peak"));
    }

    #[dash_test]
    fn state_lists_one_monitors_transitions_oldest_first() {
        let events = json!({"events": [
            {"monitor_id": 130136039, "date_happened": 1794449459, "alert_type": "success", "title": "[Recovered on {command:serve}]"},
            {"monitor_id": 1, "date_happened": 1794440000, "alert_type": "error", "title": "another monitor"},
            {"monitor_id": 130136039, "date_happened": 1794447779, "alert_type": "error", "title": "[Triggered on {command:serve}]"}
        ]});
        assert_eq!(
            transitions(130136039, &events),
            [
                "2026-11-12T01:42:59Z error: [Triggered on {command:serve}]",
                "2026-11-12T02:10:59Z success: [Recovered on {command:serve}]"
            ]
        );
    }

    #[dash_test]
    fn the_baseline_is_a_median_and_max_by_scope() {
        let answer = json!({"series": [
            {"scope": "command:serve,service:dash-rust", "pointlist": [[1, 60.0], [2, null], [3, 90.0], [4, 70.0]]},
            {"scope": "command:alerts,service:dash-rust", "pointlist": [[1, null]]}
        ]});
        assert_eq!(
            baseline(&answer),
            "  command:serve,service:dash-rust: median 70.0, max 90.0, 3 points\n"
        );
    }
}
