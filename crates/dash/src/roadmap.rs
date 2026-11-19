//! The roadmap as Linear's projects (task linear-projects-view; @nha, 2026-11-11: « /plan should have a view like linear
//! projects probably as well. They have clear dependencies. », « "The roadmap" doesn't make sense. Isn't the roadmap
//! now/next/later ? »). An item is the older app's focus row (title, horizon, covers) with its plan, Rust's own (start, target,
//! depends-on, milestones: `dash roadmap set`), and its numbers from its tasks. /plan draws them on the design system's Gantt,
//! grouped Urgent, Now, Next, Later, a row opening its panel; `dash linear roadmap` writes the plan's milestones and
//! dependencies to the items' Linear projects; `dash linear sync` (linear.rs, task rust-linear-sync-port) writes the projects
//! themselves, their status and dates, then calls it.
use dash_surface::Drawing;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

pub use dash_store::roadmap::{HORIZONS, Item, items};

/// `v` joined by `sep`, or « none ».
fn none_or(v: &[impl AsRef<str>], sep: &str) -> String {
    let v: Vec<&str> = v.iter().map(AsRef::as_ref).collect();
    if v.is_empty() {
        "none".into()
    } else {
        v.join(sep)
    }
}

/// Where a row opens its panel.
fn opens(id: &str) -> String {
    format!("/plan?item={id}")
}

/// /plan's tree: the items on one Gantt, grouped Urgent, Now, Next, Later, each followed by its milestones; `open`, the item
/// whose panel shows under it. The page is the roadmap: no heading of its own above the Gantt.
pub fn tree(items: &[Item], open: Option<&str>, today: i64) -> Drawing {
    let alarms = dash_store::roadmap::alarms(items);
    let mut elements = Map::new();
    elements.insert(
        "title".into(),
        json!({"type": "heading", "props": {"text": "Plan"}}),
    );
    let mut children = vec!["title".to_string()];
    let mut rows = vec![];
    // by its group, so a horizon the four do not name is drawn where `dash roadmap show` puts it, under Later (/code-review)
    for (_, group) in HORIZONS {
        for i in items.iter().filter(|i| i.group() == group) {
            let late = !i.complete()
                && i.target()
                    .and_then(dash_core::days)
                    .is_some_and(|t| t < today);
            let alarmed = alarms.iter().any(|a| a.id == i.item.id);
            let mark = match (i.complete() && !alarmed, late || alarmed) {
                (true, _) => "done",
                (_, true) => "failed",
                _ => "next",
            };
            let (start, target) = i.drawn(today);
            rows.push(json!({"id": i.item.id, "name": i.item.title, "start": start,
                "target": target.unwrap_or_default(), "after": i.plan.after, "mark": mark, "group": group,
                "href": opens(&i.item.id)}));
            // « / » is in no item id, so a milestone's never takes an item's (/code-review)
            for (n, (day, name)) in i.plan.milestones.iter().enumerate() {
                rows.push(
                    json!({"id": format!("{}/m{n}", i.item.id), "name": name, "target": day,
                    "mark": i.milestone(day, today), "group": group, "href": opens(&i.item.id)}),
                );
            }
        }
    }
    match rows.is_empty() {
        true => {
            let none = "No roadmap item yet: dash focus set adds one.";
            elements.insert(
                "gantt".into(),
                json!({"type": "text", "props": {"text": none}}),
            );
        }
        false => {
            elements.insert(
                "gantt".into(),
                json!({"type": "gantt", "props": {"rows": rows}}),
            );
        }
    }
    children.push("gantt".into());
    // a line under the Gantt, when it has something to say
    let mut line = |id: &str, head: &str, sep: &str, said: Vec<&str>| {
        if !said.is_empty() {
            let text = format!("{head}: {}", said.join(sep));
            elements.insert(id.into(), json!({"type": "text", "props": {"text": text}}));
            children.push(id.into());
        }
    };
    // red on its row, and said under the Gantt (task roadmap-measures)
    line(
        "alarms",
        "Alarm",
        "; ",
        alarms.iter().map(|a| a.said.as_str()).collect(),
    );
    // what the roadmap lacks, as its old page said it: an item but an Urgent one links its RFC (rfc-agents-and-limits 3.4)
    let no_rfc: Vec<&str> = (items.iter())
        .filter(|i| i.item.horizon != "urgent" && i.rfc().is_none())
        .map(|i| i.item.id.as_str())
        .collect();
    line("no-rfc", "No RFC", ", ", no_rfc);
    if let Some(i) = open.and_then(|o| items.iter().find(|i| i.item.id == o)) {
        children.push(panel(&mut elements, i, today));
    }
    elements.insert(
        "page".into(),
        json!({"type": "stack", "children": children}),
    );
    Drawing::Tree {
        title: "Plan".into(),
        tree: json!({"root": "page", "elements": elements}).to_string(),
    }
}

/// An item's panel, Linear's project properties: status, priority, lead, dates, progress, what it waits on and its definition
/// of done, then its milestones on the Timeline.
fn panel(elements: &mut Map<String, Value>, i: &Item, today: i64) -> String {
    let day = |d: Option<&str>| d.unwrap_or("Not set").to_string();
    let after = match i.plan.after.is_empty() {
        true => "Nothing".to_string(),
        false => i.plan.after.join(", "),
    };
    let props = [
        ("Status", i.status().to_string()),
        ("Priority", i.priority().to_string()),
        ("Lead", i.lead().to_string()),
        ("Start", day(i.start())),
        ("Target", day(i.target())),
        ("Progress", i.progress_said()),
        ("Blocked by", after),
        (
            "Done when",
            i.item.dod.clone().unwrap_or_else(|| "Not set yet".into()),
        ),
    ];
    let mut rows: Vec<Value> = props
        .iter()
        .map(|(k, v)| json!({"cells": [k, v]}))
        .collect();
    // its tasks, its page and its RFC, as the old /plan's card linked them
    rows.push(json!({"cells": ["Tasks", i.item.tasks.to_string()]}));
    let page = format!("/focus/{}", i.item.id);
    rows.push(json!({"cells": ["Page", page], "href": page}));
    rows.push(match i.rfc() {
        Some(rfc) => json!({"cells": ["RFC", rfc], "href": rfc}),
        None => json!({"cells": ["RFC", "None yet"]}),
    });
    elements.insert(
        "panel-props".into(),
        json!({"type": "table", "props": {"columns": ["Property", "Value"], "rows": rows}}),
    );
    let milestones = match i.plan.milestones.is_empty() {
        true => {
            json!({"type": "text", "props": {"text": "No milestone yet: dash roadmap set <id> --milestone '<day> <name>'."}})
        }
        false => {
            let events: Vec<Value> = (i.plan.milestones.iter())
                .map(
                    |(day, name)| json!({"at": day, "what": name, "mark": i.milestone(day, today)}),
                )
                .collect();
            json!({"type": "timeline", "props": {"events": events}})
        }
    };
    elements.insert("panel-milestones".into(), milestones);
    elements.insert(
        "panel-milestones-part".into(),
        json!({"type": "section", "props": {"title": "Milestones"}, "children": ["panel-milestones"]}),
    );
    elements.insert(
        "panel".into(),
        json!({"type": "section", "props": {"title": i.item.title},
            "children": ["panel-props", "panel-milestones-part"]}),
    );
    "panel".into()
}

/// /plan: the roadmap read now, the older app's items with Rust's plans; a read that fails is said on the page, its alert
/// in place of the Gantt, never an error page. `older`: the older app's database.
pub fn page(dir: &Path, open: Option<&str>, older: &Path) -> Result<Drawing, String> {
    let read = || {
        let plans = dash_store::tasks::read_plans(dir)?;
        items(older, &plans)
    };
    Ok(match read() {
        Ok(items) => tree(&items, open, dash_surface::components::today()),
        Err(e) => {
            let alert = json!({"type": "alert", "props": {"title": "The plan could not be read", "text": e}});
            let elements = json!({"title": {"type": "heading", "props": {"text": "Plan"}}, "read": alert,
                "page": {"type": "stack", "children": ["title", "read"]}});
            Drawing::Tree {
                title: "Plan".into(),
                tree: json!({"root": "page", "elements": elements}).to_string(),
            }
        }
    })
}

dash_core::submit! { crate::usage::Command::usage("dash roadmap set [--data <dir>] <id> [--start <day>] [--target <day>] [--after <ids>] [--milestone '<day> <name>']... [--measure open-prs --bound <n>]") }
/// `dash roadmap set [--data <dir>] <id> [--start <day>] [--target <day>] [--after <ids>] [--milestone '<day> <name>']...
/// [--measure open-prs --bound <n>]`:
/// the fields given written to item `id`'s plan; `--milestone` given replaces the list (one empty clears it), `--after` a comma
/// or space apart. `older`: the older app's database, which holds the items; `tasks`: Rust's store, which takes the plan.
pub fn set(
    tasks: &mut dash_store::tasks::SqliteTasks,
    older: &Path,
    id: &str,
    flags: &[&str],
    out: impl Write,
) -> Result<(), String> {
    let mut fields: Vec<(&str, String)> = vec![];
    let mut milestones: Option<Vec<&str>> = None;
    let mut it = flags.iter();
    while let Some(flag) = it.next() {
        let value = it.next().ok_or(format!("{flag}: its value is missing"))?;
        match *flag {
            "--start" | "--target" | "--measure" | "--bound" => {
                fields.push((&flag[2..], value.to_string()))
            }
            "--after" => fields.push(("after", value.replace(',', " "))),
            "--milestone" => milestones.get_or_insert_default().push(value),
            _ => {
                return Err(format!(
                    "{flag}: dash roadmap set takes --start, --target, --after, --milestone, --measure and --bound"
                ));
            }
        }
    }
    if let Some(m) = milestones {
        fields.push(("milestones", m.join("\n")));
    }
    if fields.is_empty() {
        return Err("dash roadmap set <id>: name a field to set".into());
    }
    let known = dash_store::older::roadmap_items(older)?;
    let ids: Vec<&str> = known.iter().map(|i| i.id.as_str()).collect();
    let unknown = std::iter::once(id).chain(
        fields
            .iter()
            .filter(|f| f.0 == "after")
            .flat_map(|f| f.1.split_whitespace()),
    );
    if let Some(odd) = unknown.into_iter().find(|i| !ids.contains(i)) {
        return Err(format!(
            "{odd}: no roadmap item has this id (dash focus show lists them)"
        ));
    }
    // every field at once: one that does not read leaves the plan as it was (/code-review)
    let given: Vec<(&str, &str)> = fields.iter().map(|(f, v)| (*f, v.as_str())).collect();
    tasks.plan(id, &given)?;
    let mut out = out;
    writeln!(
        out,
        "{id}: plan set ({})",
        fields.iter().map(|f| f.0).collect::<Vec<_>>().join(", ")
    )
    .map_err(|e| e.to_string())
}

dash_core::submit! { crate::usage::Command::usage("dash roadmap show [--data <dir>]") }
/// `dash roadmap show [--data <dir>]`: each item a record, its fields a tab apart: id, group, status, priority, start, target,
/// progress, what it waits on, its milestones, its measure (`Plan::measured`).
pub fn show(dir: &Path, older: &Path, out: impl Write) -> Result<(), String> {
    let plans = dash_store::tasks::read_plans(dir)?;
    let mut out = out;
    for i in items(older, &plans)? {
        let milestones: Vec<String> = i
            .plan
            .milestones
            .iter()
            .map(|(d, n)| format!("{d} {n}"))
            .collect();
        let fields = [
            i.item.id.clone(),
            i.group().into(),
            i.status().into(),
            i.priority().into(),
            format!("start {}", i.start().unwrap_or("none")),
            format!("target {}", i.target().unwrap_or("none")),
            i.progress_said(),
            format!("after {}", none_or(&i.plan.after, ", ")),
            format!("milestones {}", none_or(&milestones, "; ")),
            i.plan.measured(),
        ];
        writeln!(out, "{}", fields.join("\t")).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A GraphQL call to Linear: the query and its variables in, `data` out, or the error said.
pub type Post<'a> = dyn FnMut(&str, Value) -> Result<Value, String> + 'a;

/// An item's Linear project, named as the older app's sync names it (`Dashboard.Linear.roadmap_name/1`).
pub fn project_name(i: &Item) -> String {
    format!("Roadmap: {}", i.item.title)
}

/// The roadmap projects a page at a time; a project's milestones and relations past one page are refused, never read as
/// missing (/code-review). 25 a page: 50 cost 15,120 of Linear's 10,000 complexity points (live run, 2026-11-11).
const PROJECTS: &str = "query($after: String) { projects(first: 25, after: $after, filter: {name: {startsWith: \"Roadmap: \"}}) { \
    pageInfo { hasNextPage endCursor } nodes { id name \
    projectMilestones(first: 25) { pageInfo { hasNextPage } nodes { id name targetDate } } \
    relations(first: 25) { pageInfo { hasNextPage } nodes { anchorType relatedAnchorType project { id } relatedProject { id } } } } } }";

/// Each roadmap project by name: its id, its milestones (name, id, day) and the projects it blocks.
type Held = HashMap<String, (String, Vec<(String, String, Option<String>)>, Vec<String>)>;

fn held(post: &mut Post) -> Result<Held, String> {
    let text = |v: &Value| v.as_str().map(String::from);
    let more = |v: &Value| v["pageInfo"]["hasNextPage"].as_bool() == Some(true);
    let mut held = Held::new();
    let mut after = Value::Null;
    loop {
        let data = post(PROJECTS, json!({"after": after}))?;
        let page = &data["projects"];
        for p in page["nodes"].as_array().into_iter().flatten() {
            let (Some(id), Some(name)) = (text(&p["id"]), text(&p["name"])) else {
                continue;
            };
            if more(&p["projectMilestones"]) || more(&p["relations"]) {
                return Err(format!(
                    "linear: {name} holds more than 25 milestones or relations, past what this reads"
                ));
            }
            let milestones = (p["projectMilestones"]["nodes"]
                .as_array()
                .into_iter()
                .flatten())
            .filter_map(|m| Some((text(&m["name"])?, text(&m["id"])?, text(&m["targetDate"]))))
            .collect();
            // the dependencies it is the source of, its end before the related one's start: any other relation is not one
            let blocks = (p["relations"]["nodes"].as_array().into_iter().flatten())
                .filter(|r| r["project"]["id"].as_str() == Some(&id))
                .filter(|r| r["anchorType"] == "end" && r["relatedAnchorType"] == "start")
                .filter_map(|r| text(&r["relatedProject"]["id"]))
                .collect();
            held.insert(name, (id, milestones, blocks));
        }
        match (more(page), page["pageInfo"]["endCursor"].as_str()) {
            (true, Some(cursor)) => after = json!(cursor),
            _ => return Ok(held),
        }
    }
}

/// `dash linear roadmap`: each item's milestones and dependencies written to its Linear project, then read back. A milestone
/// missing is made, one at another day moved; a dependency missing is made end to start, the item it waits on its source, as
/// Linear's timeline draws it. Nothing is deleted. An item whose project the older app's sync has not made yet is said and
/// left. Its lines: each item, what was written, then what Linear holds; an error when Linear does not hold what was sent.
pub fn write_linear(items: &[Item], post: &mut Post) -> Result<Vec<String>, String> {
    let before = held(post)?;
    let project = |held: &Held, id: &str| {
        let i = items.iter().find(|i| i.item.id == id)?;
        held.get(&project_name(i)).map(|p| p.0.clone())
    };
    let mut lines = vec![];
    let planned =
        || (items.iter()).filter(|i| !i.plan.after.is_empty() || !i.plan.milestones.is_empty());
    for i in planned() {
        let Some((pid, milestones, _)) = before.get(&project_name(i)) else {
            lines.push(format!(
                "{}: no Linear project yet, the next dash linear sync makes it",
                i.item.id
            ));
            continue;
        };
        let mut wrote = vec![];
        for (day, name) in &i.plan.milestones {
            match milestones.iter().find(|m| &m.0 == name) {
                Some((_, _, Some(d))) if d == day => {}
                Some((_, mid, _)) => {
                    let q = "mutation($id: String!, $in: ProjectMilestoneUpdateInput!) { projectMilestoneUpdate(id: $id, input: $in) { success } }";
                    post(q, json!({"id": mid, "in": {"targetDate": day}}))?;
                    wrote.push(format!("milestone {name} moved to {day}"));
                }
                None => {
                    let q = "mutation($in: ProjectMilestoneCreateInput!) { projectMilestoneCreate(input: $in) { success } }";
                    post(
                        q,
                        json!({"in": {"projectId": pid, "name": name, "targetDate": day}}),
                    )?;
                    wrote.push(format!("milestone {name} {day} made"));
                }
            }
        }
        for a in &i.plan.after {
            let Some(source) = project(&before, a) else {
                wrote.push(format!("waits on {a}: its project is not on Linear yet"));
                continue;
            };
            if before.values().any(|p| p.0 == source && p.2.contains(pid)) {
                continue;
            }
            let q = "mutation($in: ProjectRelationCreateInput!) { projectRelationCreate(input: $in) { success } }";
            let input = json!({"type": "dependency", "projectId": source, "anchorType": "end",
                "relatedProjectId": pid, "relatedAnchorType": "start"});
            post(q, json!({"in": input}))?;
            wrote.push(format!("waits on {a} made"));
        }
        let wrote = if wrote.is_empty() {
            "nothing to write".into()
        } else {
            wrote.join(", ")
        };
        lines.push(format!("{}: {wrote}", i.item.id));
    }
    // read back: what Linear holds now, each item's milestones and what it waits on
    let after = held(post)?;
    let mut missing = vec![];
    for i in planned() {
        let Some((pid, milestones, _)) = after.get(&project_name(i)) else {
            continue;
        };
        let ms: Vec<String> = milestones
            .iter()
            .map(|m| format!("{} {}", m.2.as_deref().unwrap_or("no day"), m.0))
            .collect();
        let waits: Vec<&str> = (i.plan.after.iter())
            .filter(|a| {
                project(&after, a)
                    .is_some_and(|s| after.values().any(|p| p.0 == s && p.2.contains(pid)))
            })
            .map(String::as_str)
            .collect();
        lines.push(format!(
            "{}: Linear holds milestones {}; waits on {}",
            i.item.id,
            none_or(&ms, "; "),
            none_or(&waits, ", ")
        ));
        for (day, name) in &i.plan.milestones {
            if !milestones
                .iter()
                .any(|m| &m.0 == name && m.2.as_deref() == Some(day))
            {
                missing.push(format!("{}'s milestone {name} {day}", i.item.id));
            }
        }
        for a in &i.plan.after {
            if project(&after, a).is_some() && !waits.contains(&a.as_str()) {
                missing.push(format!("{} waits on {a}", i.item.id));
            }
        }
    }
    if lines.is_empty() {
        lines.push(format!(
            "No item's plan holds a milestone or a wait yet: nothing to write ({} roadmap projects read on Linear)",
            after.len()
        ));
    }
    match missing.is_empty() {
        true => Ok(lines),
        false => Err(format!(
            "{}\nLinear does not hold: {}",
            lines.join("\n"),
            missing.join(", ")
        )),
    }
}

/// Linear's API through curl, the key from bin/secret on curl's stdin, never in its arguments.
pub fn linear_post() -> Result<impl FnMut(&str, Value) -> Result<Value, String>, String> {
    let secret = crate::pages::repo().join("bin/secret");
    let key = std::process::Command::new(&secret)
        .args(["get", "linear", "LINEAR_API_KEY"])
        .output()
        .map_err(|e| format!("{}: {e}", secret.display()))?;
    let key = String::from_utf8_lossy(&key.stdout).trim().to_string();
    if key.is_empty() {
        return Err("linear: no LINEAR_API_KEY in bin/secret linear".into());
    }
    Ok(move |query: &str, vars: Value| {
        let body = json!({"query": query, "variables": vars}).to_string();
        let mut curl = std::process::Command::new("curl")
            .args([
                "-sS",
                "--max-time",
                "60",
                "-w",
                "\n%{http_code}",
                "https://api.linear.app/graphql",
                "-H",
                "Content-Type: application/json",
                "-H",
                "@-",
                "--data-binary",
                &body,
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("curl: {e}"))?;
        if let Some(mut stdin) = curl.stdin.take() {
            writeln!(stdin, "Authorization: {key}").map_err(|e| format!("curl: {e}"))?;
        }
        let out = curl.wait_with_output().map_err(|e| format!("curl: {e}"))?;
        // a call Linear left unanswered names the call, as a timeout says nothing of which one: `dash linear sync` counts these
        // and tries them again next run, never halting on one (linear::unanswered)
        let call: String = query
            .split_once('{')
            .map_or(query, |(_, q)| q)
            .trim()
            .chars()
            .take(40)
            .collect();
        // curl gave up (no connection, its 60 s): unanswered. An answer is its body, then its status on the last line
        let cut = |s: &str| s.chars().take(200).collect::<String>();
        if !out.status.success() {
            let said = String::from_utf8_lossy(&out.stderr);
            return Err(format!(
                "linear: the API failed on « {call} »: {}",
                cut(said.trim())
            ));
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let (body, status) = text.rsplit_once('\n').unwrap_or((&text, ""));
        // a GraphQL refusal comes as JSON whatever its status (400 included); any other answer is not 200: said with its status,
        // never counted as unanswered, so an outage halts the sync as the older app's did (/code-review)
        let answer: Value = match (status, serde_json::from_str::<Value>(body)) {
            (_, Ok(answer)) if answer.get("errors").is_some() || status == "200" => answer,
            _ => return Err(format!("linear: the API answered {status}: {}", cut(body))),
        };
        // Linear's own message first: « usage limit exceeded », « Entity not found: Issue » are read by it
        match answer.get("errors") {
            Some(errors) => Err(format!(
                "linear: {}",
                errors[0]["message"]
                    .as_str()
                    .map_or(errors.to_string(), String::from)
            )),
            None => Ok(answer["data"].clone()),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;
    use dash_store::older::RoadmapItem;
    use dash_store::tasks::Plan;

    fn item(id: &str, title: &str, horizon: &str, points: i64, done: i64, plan: Plan) -> Item {
        Item {
            item: RoadmapItem {
                id: id.into(),
                title: title.into(),
                horizon: horizon.into(),
                dod: Some(format!("{title} done")),
                points,
                done,
                first: Some("2026-11-01".into()),
                last: Some("2026-11-06".into()),
                link: (id == "rust").then(|| "/v/private/docs/rfc-rust-unified.html#x".into()),
                rfc: None,
                tasks: 3,
            },
            plan,
        }
    }

    fn items() -> Vec<Item> {
        let rust = Plan {
            start: None,
            target: Some("2026-12-15".into()),
            after: vec![],
            milestones: vec![("2026-11-20".into(), "Design system".into())],
            ..Plan::default()
        };
        let jobs = Plan {
            start: Some("2026-11-10".into()),
            target: Some("2026-11-30".into()),
            after: vec!["rust".into()],
            milestones: vec![],
            ..Plan::default()
        };
        vec![
            item("jobs", "Job search", "next", 2, 1, jobs),
            item("rust", "The Rust tool builds itself", "now", 10, 4, rust),
            item("fire", "A fire", "urgent", 1, 1, Plan::default()),
            item("nano", "Nanolathe", "later", 3, 1, Plan::default()),
        ]
    }

    fn text_of(d: &Drawing) -> String {
        text(d)
    }

    fn text(d: &Drawing) -> String {
        let Drawing::Tree { tree, .. } = d else {
            panic!("a tree")
        };
        dash_surface::text(tree).expect("its text")
    }

    /// Done-when 3: /plan draws the items on the Gantt, grouped Urgent, Now, Next, Later, what waits on what its arrows; no
    /// « The roadmap » heading, the page is the roadmap.
    #[dash_test]
    fn plan_draws_the_items_on_the_gantt_grouped_by_horizon() {
        let today = dash_core::days("2026-11-11").unwrap();
        let drawn = tree(&items(), None, today);
        // the whole text, the today line Mermaid's comment of the real day: every group, Urgent, Now, Next, Later, in order,
        // each item's bar, milestone, wait and link; the RFC note after; no « The roadmap » heading
        let text = text(&drawn);
        let today_line = format!(
            "  %% today {}\n",
            dash_core::date(dash_surface::components::today())
        );
        assert_eq!(
            text.replacen(&today_line, "  %% today <real day>\n", 1),
            [
                "Plan",
                "gantt",
                "  dateFormat YYYY-MM-DD",
                "  inclusiveEndDates",
                "  %% today <real day>",
                "  section Urgent",
                "  %% fire opens /plan?item=fire",
                "  A fire :done, fire, 2026-11-01, 2026-11-06",
                "  section Now",
                "  %% rust opens /plan?item=rust",
                "  The Rust tool builds itself :rust, 2026-11-01, 2026-12-15",
                "  %% rust_m0 opens /plan?item=rust",
                "  Design system :milestone, rust_m0, 2026-11-20, 0d",
                "  section Next",
                "  %% jobs after rust",
                "  %% jobs opens /plan?item=jobs",
                "  Job search :jobs, 2026-11-10, 2026-11-30",
                "  section Later",
                "  %% nano opens /plan?item=nano",
                "  Nanolathe :nano, 2026-11-01, 2026-11-11",
                "No RFC: jobs, nano",
            ]
            .map(|l| l.to_string() + "\n")
            .concat()
        );
        // /code-review: a target alone is a one-day bar, no target runs to today, a start after its target starts on it, a
        // horizon the four do not name sits under Later, a past milestone of an unfinished item is failed
        let odd = |horizon: &str, start: Option<&str>, target: Option<&str>, points: i64| {
            let plan = Plan {
                start: start.map(String::from),
                target: target.map(String::from),
                milestones: vec![("2026-11-05".into(), "Missed".into())],
                ..Plan::default()
            };
            let mut i = item("odd", "Odd", horizon, points, 0, plan);
            i.item.first = None;
            text_of(&tree(&[i], None, today))
        };
        let t = odd("next", None, Some("2026-12-01"), 0);
        assert!(t.contains("  Odd :odd, 2026-12-01, 2026-12-01\n"), "{t}");
        let t = odd("someday", Some("2026-11-02"), None, 3);
        assert!(
            t.contains("  section Later\n") && t.contains("  Odd :odd, 2026-11-02, 2026-11-11\n"),
            "{t}"
        );
        let t = odd("now", Some("2026-12-20"), Some("2026-12-01"), 3);
        assert!(t.contains("  Odd :odd, 2026-12-01, 2026-12-01\n"), "{t}");
        assert!(
            t.contains("  Missed :crit, milestone, odd_m0, 2026-11-05, 0d\n"),
            "{t}"
        );
    }

    /// Done-when 3 on the page he sees: /plan's tree drawn by the design system's renderer, as `dash serve` answers a browser,
    /// draws the four groups' labels on the Gantt, Urgent, Now, Next, Later, in order, a row's name opening its panel, and no
    /// « The roadmap » anywhere. Read by the Gantt's own class, never a tag literal (dash-gate's the_dash_crate_holds_no_html).
    #[dash_test]
    fn plan_s_rendered_page_draws_the_four_groups_and_no_roadmap_heading() {
        let today = dash_core::days("2026-11-11").unwrap();
        let Drawing::Tree { title, tree: drawn } = tree(&items(), None, today) else {
            panic!("a tree")
        };
        let html =
            dash_surface::markup(&dash_surface::screen::page(&title, &drawn).expect("drawn"));
        // each group label's words: what follows its element's « > » up to the next « < »
        let groups: Vec<&str> = (html.split("class=\"gantt-group\"").skip(1))
            .map(|rest| {
                let words = &rest[rest.find('>').expect("its element") + 1..];
                &words[..words.find('<').expect("its end")]
            })
            .collect();
        assert_eq!(groups, ["Urgent", "Now", "Next", "Later"], "{html}");
        assert!(!html.contains("The roadmap"), "{html}");
        assert!(
            html.contains("href=\"/plan?item=nano\""),
            "a row opens its panel: {html}"
        );
        // and the page's own heading is Plan, the first words a reader meets
        assert!(
            dash_surface::words::text_of(&html).starts_with("Plan\n"),
            "{html}"
        );
    }

    /// Done-when 4: a row opens its panel: status, priority, lead, dates, milestones, progress.
    #[dash_test]
    fn a_row_opens_its_panel() {
        let today = dash_core::days("2026-11-11").unwrap();
        let text = text(&tree(&items(), Some("rust"), today));
        let panel = &text[text
            .find("\nThe Rust tool builds itself\n")
            .expect("its panel")..];
        for line in [
            "Status\tIn Progress",
            "Priority\tHigh",
            "Lead\tNicolas Ha",
            "Start\t2026-11-01",
            "Target\t2026-12-15",
            "Progress\t40 % (4 of 10 points)",
            "Blocked by\tNothing",
            "Tasks\t3",
            "Page\t/focus/rust\t/focus/rust",
            "RFC\t/v/private/docs/rfc-rust-unified.html#x\t/v/private/docs/rfc-rust-unified.html#x",
            "\nMilestones\n2026-11-20\tnext\tDesign system",
        ] {
            assert!(panel.contains(line), "{line} in {panel}");
        }
        let jobs = text_of_panel(&items(), "jobs", today);
        assert!(
            jobs.contains("Status\tPlanned") && jobs.contains("Blocked by\trust"),
            "{jobs}"
        );
        assert!(jobs.contains("No milestone yet"), "{jobs}");
        let fire = text_of_panel(&items(), "fire", today);
        assert!(
            fire.contains("Status\tCompleted") && fire.contains("Target\t2026-11-06"),
            "{fire}"
        );
        // no panel for an id no item has
        assert!(!text_of_panel(&items(), "nope", today).contains("Status\t"));
    }

    fn text_of_panel(items: &[Item], id: &str, today: i64) -> String {
        text(&tree(items, Some(id), today))
    }

    /// A stand-in Linear: its projects, milestones and relations, changed by the mutations the writer sends.
    #[derive(Default)]
    struct Linear {
        projects: Vec<(String, String)>,
        milestones: Vec<(String, String, String, String)>,
        relations: Vec<(String, String, String, String)>,
        calls: usize,
        /// Projects a page, all when 0: Linear's `first`, its cursor the next index.
        page: usize,
        /// Each mutation sent, by name, in order.
        sent: Vec<String>,
    }

    impl Linear {
        fn post(&mut self, query: &str, vars: Value) -> Result<Value, String> {
            self.calls += 1;
            let s = |v: &Value| v.as_str().unwrap_or_default().to_string();
            if query.starts_with("query") {
                let from: usize = vars["after"]
                    .as_str()
                    .map_or(0, |c| c.parse().expect("a cursor"));
                let size = if self.page == 0 {
                    self.projects.len()
                } else {
                    self.page
                };
                let to = (from + size).min(self.projects.len());
                let nodes: Vec<Value> = (self.projects[from..to].iter())
                    .map(|(id, name)| {
                        let ms: Vec<Value> = (self.milestones.iter().filter(|m| &m.1 == id))
                            .map(|m| json!({"id": m.0, "name": m.2, "targetDate": m.3}))
                            .collect();
                        let rs: Vec<Value> = (self.relations.iter().filter(|r| &r.0 == id || &r.2 == id))
                            .map(|r| json!({"project": {"id": r.0}, "anchorType": r.1, "relatedProject": {"id": r.2}, "relatedAnchorType": r.3}))
                            .collect();
                        json!({"id": id, "name": name, "projectMilestones": {"nodes": ms}, "relations": {"nodes": rs}})
                    })
                    .collect();
                let info =
                    json!({"hasNextPage": to < self.projects.len(), "endCursor": to.to_string()});
                return Ok(json!({"projects": {"pageInfo": info, "nodes": nodes}}));
            }
            let input = &vars["in"];
            // a project's own fields, its status and dates, are the older sync's: Rust never writes a project, and the only
            // date it sends is a milestone's own (his orchestrator, 2026-11-11: « Dates stay Elixir's »)
            let name = [
                "projectMilestoneCreate",
                "projectMilestoneUpdate",
                "projectRelationCreate",
            ]
            .into_iter()
            .find(|m| query.contains(&format!("{m}(")))
            .ok_or(format!("a mutation Rust may not send: {query}"))?;
            if input.get("startDate").is_some()
                || (input.get("targetDate").is_some() && !name.contains("Milestone"))
            {
                return Err(format!("a project date sent: {vars}"));
            }
            self.sent.push(name.into());
            if query.contains("projectMilestoneCreate") {
                let n = self.milestones.len();
                self.milestones.push((
                    format!("m{n}"),
                    s(&input["projectId"]),
                    s(&input["name"]),
                    s(&input["targetDate"]),
                ));
            } else if query.contains("projectMilestoneUpdate") {
                let m = self
                    .milestones
                    .iter_mut()
                    .find(|m| m.0 == s(&vars["id"]))
                    .ok_or("no milestone")?;
                m.3 = s(&input["targetDate"]);
            } else if query.contains("projectRelationCreate") {
                assert_eq!(
                    (
                        s(&input["type"]),
                        s(&input["anchorType"]),
                        s(&input["relatedAnchorType"])
                    ),
                    ("dependency".into(), "end".into(), "start".into())
                );
                self.relations.push((
                    s(&input["projectId"]),
                    s(&input["anchorType"]),
                    s(&input["relatedProjectId"]),
                    s(&input["relatedAnchorType"]),
                ));
            } else {
                return Err(format!("unknown {query}"));
            }
            Ok(json!({"ok": true}))
        }
    }

    /// Done-when 2: dash linear roadmap writes each item's milestones and what it waits on to its Linear project, as a
    /// dependency from the end of the one it waits on to its start; the API reads them back. A second run writes nothing.
    #[dash_test]
    fn linear_holds_the_milestones_and_dependencies_written() {
        let items = items();
        let mut linear = Linear {
            projects: vec![
                (
                    "p-rust".into(),
                    "Roadmap: The Rust tool builds itself".into(),
                ),
                ("p-jobs".into(), "Roadmap: Job search".into()),
            ],
            // a milestone held at another day is moved, never made twice
            milestones: vec![(
                "m-old".into(),
                "p-rust".into(),
                "Design system".into(),
                "2026-11-18".into(),
            )],
            // a relation from rust's start is not the dependency jobs needs: it is made beside it
            relations: vec![(
                "p-rust".into(),
                "start".into(),
                "p-jobs".into(),
                "start".into(),
            )],
            // one project a page: the second is on the next one
            page: 1,
            ..Linear::default()
        };
        let lines =
            write_linear(&items, &mut |q: &str, v: Value| linear.post(q, v)).expect("written");
        assert_eq!(
            lines,
            [
                "jobs: waits on rust made",
                "rust: milestone Design system moved to 2026-11-20",
                "jobs: Linear holds milestones none; waits on rust",
                "rust: Linear holds milestones 2026-11-20 Design system; waits on none",
            ]
        );
        assert_eq!(
            linear.relations[1..],
            [(
                "p-rust".into(),
                "end".into(),
                "p-jobs".into(),
                "start".into()
            )]
        );
        assert_eq!(linear.milestones.len(), 1);
        // the mutations sent: a relation and a milestone's own date, never a project's start or target (the stand-in refuses
        // any project write or project date, so the run above would have failed on one)
        assert_eq!(
            linear.sent,
            ["projectRelationCreate", "projectMilestoneUpdate"]
        );
        let calls = linear.calls;
        let again =
            write_linear(&items, &mut |q: &str, v: Value| linear.post(q, v)).expect("again");
        assert_eq!(
            &again[..2],
            ["jobs: nothing to write", "rust: nothing to write"]
        );
        assert_eq!(
            linear.calls - calls,
            4,
            "a second run only reads, before and after, two pages each"
        );
        // an item whose project the older sync has not made is said, nothing sent for it
        linear.projects.retain(|p| p.0 != "p-jobs");
        let lines =
            write_linear(&items, &mut |q: &str, v: Value| linear.post(q, v)).expect("one missing");
        assert_eq!(
            lines[0],
            "jobs: no Linear project yet, the next dash linear sync makes it"
        );
        // no plan to write is said, never silence
        let bare: Vec<Item> = (items.iter())
            .map(|i| Item {
                plan: Plan::default(),
                ..i.clone()
            })
            .collect();
        let lines =
            write_linear(&bare, &mut |q: &str, v: Value| linear.post(q, v)).expect("nothing");
        assert_eq!(
            lines,
            [
                "No item's plan holds a milestone or a wait yet: nothing to write (1 roadmap projects read on Linear)"
            ]
        );
        // Linear that drops a write is an error, named
        let mut deaf = |q: &str, v: Value| match q.starts_with("query") {
            true => Linear {
                projects: vec![(
                    "p-rust".into(),
                    "Roadmap: The Rust tool builds itself".into(),
                )],
                ..Linear::default()
            }
            .post(q, v),
            false => Ok(json!({})),
        };
        let err = write_linear(&items, &mut deaf).expect_err("not held");
        assert!(
            err.ends_with("Linear does not hold: rust's milestone Design system 2026-11-20"),
            "{err}"
        );
    }

    /// Each pass as the stall watcher runs it (dash_store::roadmap::pass): what an item reads kept as its seen, each wake landed
    /// once by its key (dash_comments::watch::land, whose own test holds the once). The keys that woke.
    fn passes(items: &mut [Item], counts: &[i64]) -> Vec<String> {
        let mut woke: Vec<String> = vec![];
        for (n, count) in counts.iter().enumerate() {
            let mut read = vec![];
            let mut write = |id: &str, f: &[(&str, &str)]| {
                let f: Vec<(String, String)> = f
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect();
                read.push((id.to_string(), f));
                Ok(())
            };
            let mut wake = |key: &str, _: &str| {
                woke.push(key.into());
                Ok(true)
            };
            let (_, failed) =
                dash_store::roadmap::pass(items, *count, n as i64, &mut write, &mut wake);
            assert_eq!(failed, None);
            for (id, fields) in read {
                let plan = &mut items
                    .iter_mut()
                    .find(|i| i.item.id == id)
                    .expect("an item")
                    .plan;
                for (k, v) in fields {
                    match k.as_str() {
                        "seen" => plan.seen = v.parse().ok(),
                        _ => plan.told = v.split_whitespace().map(String::from).collect(),
                    }
                }
            }
        }
        woke
    }

    /// Done-when 1 (task roadmap-measures): an Urgent item with no task, then one with no definition of done, is red on /plan
    /// (its Gantt row failed, the alarm said under it) and wakes the orchestrator once over three passes.
    #[dash_test]
    fn an_urgent_item_with_no_task_or_no_dod_is_red_and_wakes_once() {
        let today = dash_core::days("2026-11-11").expect("a day");
        let mut arrow = item("arrow", "Arrow call", "urgent", 0, 0, Plan::default());
        arrow.item.tasks = 0;
        let t = text_of(&tree(&[arrow.clone()], None, today));
        assert!(t.contains("  Arrow call :crit, arrow,"), "{t}");
        assert!(
            t.contains("Alarm: Urgent Arrow call (arrow) has no task linked"),
            "{t}"
        );
        assert_eq!(
            passes(&mut [arrow.clone()], &[3, 3, 3]),
            ["roadmap arrow no-task 0"]
        );
        arrow.item.tasks = 2;
        arrow.item.dod = None;
        let t = text_of(&tree(&[arrow.clone()], None, today));
        assert!(
            t.contains("Alarm: Urgent Arrow call (arrow) has no definition of done"),
            "{t}"
        );
        assert_eq!(
            passes(&mut [arrow.clone()], &[3, 3]),
            ["roadmap arrow no-dod 0"]
        );
        // with both, no alarm: not red, no wake, and its told cleared
        arrow.item.dod = Some("Flare ready".into());
        let t = text_of(&tree(&[arrow.clone()], None, today));
        assert!(!t.contains("Alarm") && !t.contains(":crit, arrow"), "{t}");
        let mut one = [arrow];
        assert!(passes(&mut one, &[3]).is_empty());
        // /code-review: the gap coming back wakes again
        one[0].item.tasks = 0;
        assert_eq!(passes(&mut one, &[3, 3]), ["roadmap arrow no-task 0"]);
    }

    /// Done-when 2: a Now item measuring open PRs, bound 8, wakes once when 9 are open, not again while they stay over, and
    /// once more only after it came back under; /plan shows it red while over.
    #[dash_test]
    fn a_now_item_past_its_bound_wakes_once_per_crossing() {
        let today = dash_core::days("2026-11-11").expect("a day");
        let plan = Plan {
            measure: Some("open-prs".into()),
            bound: Some(8),
            ..Plan::default()
        };
        let mut pile = [item("pr-pile", "Never piled", "now", 4, 1, plan)];
        assert_eq!(
            passes(&mut pile, &[5, 9, 12, 9]),
            ["roadmap pr-pile open-prs 1"]
        );
        assert_eq!(pile[0].plan.seen, Some(9));
        let t = text_of(&tree(&pile, None, today));
        assert!(
            t.contains("Alarm: Never piled (pr-pile): open-prs reads 9, over its bound of 8"),
            "{t}"
        );
        assert!(t.contains("  Never piled :crit, pr-pile,"), "{t}");
        assert_eq!(
            passes(&mut pile, &[8, 9]),
            ["roadmap pr-pile open-prs 1"],
            "under then over again: a new crossing"
        );
        // at its bound: not over, not red
        pile[0].plan.seen = Some(8);
        pile[0].plan.told = vec![];
        assert!(!text_of(&tree(&pile, None, today)).contains("Alarm"));
        // /code-review: a bound lowered under the reading is a crossing too
        pile[0].plan.bound = Some(20);
        assert!(passes(&mut pile, &[10]).is_empty());
        pile[0].plan.bound = Some(8);
        assert_eq!(passes(&mut pile, &[10]), ["roadmap pr-pile open-prs 0"]);
        // /code-review: a wake that fails is not told, so the next pass tries it again
        pile[0].plan.told = vec![];
        let mut wake = |_: &str, _: &str| Err("the log is locked".into());
        let (_, failed) = dash_store::roadmap::pass(&pile, 11, 0, &mut |_, _| Ok(()), &mut wake);
        assert!(failed.is_some());
        assert_eq!(passes(&mut pile, &[11]), ["roadmap pr-pile open-prs 0"]);
    }
}
