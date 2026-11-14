//! `dash audit` and /tools' calls (task rust-audit-log; @nha, 2026-11-12: « we should recall what works and what doesn't »):
//! what the core's tool port kept, summed by tool over a period, from one tree for the page and the command.
use dash_core::registry::{Call, Registry};
use dash_store::audit::Sum;
use serde_json::{Value, json};
use std::path::Path;

const DAY_MS: i64 = 86_400_000;

/// The audit log when the store cannot open: each record fails with why, which tools_call says on stderr. None: a surface's
/// log, which keeps no call and fails none.
pub struct Unkept(pub Option<String>);

impl dash_core::registry::Audit for Unkept {
    fn record(&self, _call: &Call) -> Result<(), String> {
        self.0
            .as_ref()
            .map_or(Ok(()), |e| Err(format!("the store did not open: {e}")))
    }
}

/// `dir`'s audit log: a store that cannot open keeps no call, and every tool still answers (/code-review).
pub fn kept(dir: &Path) -> Box<dyn dash_core::registry::Audit> {
    dash_store::open(dir).map_or_else(
        |e| Box::new(Unkept(Some(e.to_string()))) as _,
        |s| Box::new(s) as _,
    )
}

/// `dash tools <tool>` and its card on /tools: its usage left, asked through the core's port, so `dir`'s audit log keeps the
/// call like any other (task rust-tool-hostman).
pub fn usage_left(registry: &Registry, dir: &Path, name: &str) -> Result<String, String> {
    // a name no tool holds says so, never « it reads no usage » (/code-review)
    let tool = registry.tools.get(name).ok_or("no such tool")?;
    let asked = tool.usage().ok_or("it reads no usage")?;
    let now = crate::tasks::now_ms;
    let (answer, recorded) = registry
        .call(name, asked, &now, &*kept(dir))
        .expect("the tool found above");
    // said on stderr, as tools_call says it: the reading still answers (/code-review)
    if let Err(e) = recorded {
        eprintln!("audit: {name}'s call not kept: {e}");
    }
    answer
}

/// The start of `day` (2026-11-12) in Unix milliseconds, at midnight in a zone `offset_ms` ahead of UTC.
pub fn day_ms(day: &str, offset_ms: i64) -> Result<i64, String> {
    dash_core::days(day)
        .filter(|_| dash_core::is_day(day))
        .map(|d| d * DAY_MS - offset_ms)
        .ok_or(format!("{day}: not a day, as 2026-11-12"))
}

/// `day_ms` at his midnight, as `today` names his day: a UTC midnight counted no call of the hour past his, so « since today »
/// read none from 00:00 to 01:00 in Paris (rust-rejudge-merged-closes' pre-push, 2026-11-13 00:03).
fn his_day_ms(day: &str) -> Result<i64, String> {
    day_ms(day, crate::pages::utc_offset_ms())
}

/// Today in his zone, as the board's today (/code-review).
pub fn today(now_ms: i64) -> String {
    dash_core::date(crate::tasks::today_of(now_ms))
}

/// The store in `data` for reading; none while no file is there, which is no call yet (/code-review: /tools and `dash audit`
/// said it two ways).
fn read(data: &Path) -> Result<Option<dash_store::SqliteStore>, String> {
    if !data.join("dash.sqlite").exists() {
        return Ok(None);
    }
    dash_store::open_read(data)
        .map(Some)
        .map_err(|e| e.to_string())
}

fn sums(data: &Path, tool: Option<&str>, since_ms: i64) -> Result<Vec<Sum>, String> {
    read(data)?.map_or(Ok(vec![]), |s| s.audit_sums(tool, since_ms))
}

/// Each tool's sum as a table row; a live tool with no call is a row of zeros, so « no call » never reads as « no tool ».
fn sums_table(sums: &[Sum], live: &[String]) -> Value {
    let rows: Vec<Value> = sums
        .iter()
        .map(|s| {
            let last = s.last_failure.as_deref().unwrap_or("");
            let cells = [
                s.tool.clone(),
                s.worked.to_string(),
                s.failed.to_string(),
                format!("{}", s.cost),
                format!("{} ms", s.ms),
                last.to_string(),
            ];
            (s.tool.as_str(), json!({ "cells": cells }))
        })
        .chain(
            live.iter()
                .filter(|t| !sums.iter().any(|s| &s.tool == *t))
                .map(|t| (t.as_str(), json!({"cells": [t, "0", "0", "0", "0 ms", ""]}))),
        )
        .collect::<std::collections::BTreeMap<_, _>>()
        .into_values()
        .collect();
    let columns = ["Tool", "Worked", "Failed", "Cost", "Time", "Last failure"];
    json!({"type": "table", "props": {"columns": columns, "rows": rows}})
}

fn calls_table(calls: &[Call]) -> Value {
    let rows: Vec<Value> = calls
        .iter()
        .map(|c| {
            let (said, answer) = match &c.answer {
                Ok(a) => ("worked", a),
                Err(e) => ("failed", e),
            };
            let cells = [
                dash_core::stamp(c.at_ms),
                c.tool.clone(),
                said.to_string(),
                format!("{} ms", c.ms),
                format!("{}", c.cost),
                c.asked.clone(),
                answer.clone(),
            ];
            json!({ "cells": cells })
        })
        .collect();
    let columns = ["When", "Tool", "Said", "Time", "Cost", "Asked", "Answer"];
    json!({"type": "table", "props": {"columns": columns, "rows": rows}})
}

/// /tools' part: each tool's calls since `since`, a section the page adds to its own tree.
pub fn section(sums: &[Sum], live: &[String], since: &str) -> (Value, Value) {
    let title = format!("Calls since {since}");
    (
        json!({"type": "section", "props": {"title": title}, "children": ["calls-sums"]}),
        sums_table(sums, live),
    )
}

/// /tools' tree with its calls part after the title, read from the store in `data`: a store that cannot be read is said in
/// the part, and the rest of the page still draws. It names the tools with calls kept: the page builds no registry, so a tool
/// never called shows in `dash audit` alone.
pub fn on_tools(tree: &str, data: &Path, today: i64) -> String {
    let Ok(mut tree) = serde_json::from_str::<Value>(tree) else {
        return tree.to_string();
    };
    let since = dash_core::date(today - 6);
    let (part, table) = match his_day_ms(&since).and_then(|ms| sums(data, None, ms)) {
        Ok(sums) => section(&sums, &[], &since),
        Err(e) => (
            json!({"type": "section", "props": {"title": "Calls"}, "children": ["calls-sums"]}),
            json!({"type": "text", "props": {"text": format!("The audit log cannot be read: {e}")}}),
        ),
    };
    tree["elements"]["calls"] = part;
    tree["elements"]["calls-sums"] = table;
    if let Some(page) = tree["elements"]["page"]["children"].as_array_mut() {
        page.insert(page.len().min(1), json!("calls"));
    }
    tree.to_string()
}

/// `dash audit <tool> [--since <day>]`: that tool's sum since the day, today by default.
pub fn sum(data: &Path, tool: &str, since: &str) -> Result<String, String> {
    let sums = sums(data, Some(tool), his_day_ms(since)?)?;
    let tree = json!({"root": "page", "elements": {
        "page": {"type": "stack", "children": ["calls"]},
        "calls": section(&sums, &[tool.to_string()], since).0,
        "calls-sums": sums_table(&sums, &[tool.to_string()]),
    }});
    dash_surface::text(&tree.to_string())
}

/// `dash audit [--since <day>]`: every live tool's sum, then each call since the day.
pub fn day(data: &Path, live: &[String], since: &str) -> Result<String, String> {
    let ms = his_day_ms(since)?;
    let calls = read(data)?.map_or(Ok(vec![]), |s| s.audit_calls(ms))?;
    let sums = sums(data, None, ms)?;
    let tree = json!({"root": "page", "elements": {
        "page": {"type": "stack", "children": ["calls", "each"]},
        "calls": section(&sums, live, since).0,
        "calls-sums": sums_table(&sums, live),
        "each": {"type": "section", "props": {"title": "Each call"}, "children": ["each-call"]},
        "each-call": calls_table(&calls),
    }});
    dash_surface::text(&tree.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_core::registry::Audit;
    use dash_meter::TestDir;
    use dash_meter::dash_test;

    fn call(tool: &str, answer: Result<&str, &str>, at_ms: i64) -> Call {
        Call {
            tool: tool.into(),
            origin: "plugin".into(),
            asked: r#"{"q":1}"#.into(),
            answer: answer.map(str::to_string).map_err(str::to_string),
            at_ms,
            ms: 12,
            cost: 1.5,
        }
    }

    /// Task rust-audit-log, part 2: /tools shows each tool's calls of the week, what worked and what failed, in both forms.
    #[dash_test]
    fn tools_shows_what_worked_and_failed_this_week() {
        let dir = TestDir::new("audit-tools");
        let day = dash_core::days("2026-11-12").unwrap();
        let now = day * DAY_MS + 3_600_000;
        let page = r#"{"root":"page","elements":{"page":{"type":"stack","children":["title"]},"title":{"type":"heading","props":{"text":"Tools"}}}}"#;
        let none = dash_surface::text(&on_tools(page, &dir.0, day)).unwrap();
        assert!(none.contains("Calls since 2026-11-06"), "{none}");
        let store = dash_store::open(&dir.0).unwrap();
        for c in [
            call("hostman", Ok("up"), now - 7 * DAY_MS),
            call("hostman", Ok("up"), now),
            call("hostman", Err("402 no credit"), now + 1),
        ] {
            store.record(&c).unwrap();
        }
        let tree = on_tools(page, &dir.0, day);
        let text = dash_surface::text(&tree).unwrap();
        assert!(
            text.find("Tools") < text.find("Calls since 2026-11-06"),
            "{text}"
        );
        assert!(
            text.contains("hostman\t1\t1\t3\t24 ms\t402 no credit"),
            "{text}"
        );
        let html = dash_surface::screen::page("Tools", &tree).unwrap();
        assert!(
            html.contains("402 no credit") && html.contains("Calls since 2026-11-06"),
            "{html}"
        );
    }

    #[dash_test]
    fn a_day_is_written_whole() {
        assert_eq!(day_ms("1970-01-02", 0), Ok(DAY_MS));
        // Paris in winter: its midnight is 23:00 UTC the day before
        assert_eq!(day_ms("1970-01-02", 3_600_000), Ok(DAY_MS - 3_600_000));
        assert!(day_ms("2026-02-31", 0).is_err() && day_ms("2026-11-1", 0).is_err());
    }
}
