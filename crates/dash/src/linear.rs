//! `dash linear sync` (task rust-linear-sync-port; @nha, 2026-11-11: « stop falling back altogether and treat each need as the
//! impetus to build the tracer bullet replacement »): every task of the list as an issue of his Linear team AAA, every roadmap
//! item as a « Roadmap: <title> » project with its status, dates, lead, milestones and dependencies, through Linear's GraphQL
//! API. It was `Dashboard.Linear.sync/1`; what that module learned is kept here, each rule beside the code it shapes:
//! - An issue is ours by the last line of its description, « site_perso: `<key>` »: a second run finds it, so it runs again at
//!   any time. It never deletes.
//! - The free plan holds 250 issues not archived: the sync keeps `ROOM`, the open tasks ranked first (`room`), and archives
//!   a closed one once the running cycle no longer counts it, or past `KEEP_DAYS`.
//! - Each PR the board links to a task is attached once: an open closing one through his GitHub integration, the rest as a
//!   plain link whose subtitle is the PR's state.
//! - linear.edn keeps each issue's identifier and url, so a page and a brief name it with no call.
//!
//! Left with the older module: its six example issues (made once, kept as they are), the retirement of the labels its first
//! run made from epics (done since 2026-10-21), and `dash linear only|waiting|past`, which only read or attach.
use crate::roadmap::{self, Item, Post};
use dash_store::older::{LinearTask, LinkedPr};
use regex::Regex;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::LazyLock;

/// The team our tasks sync to.
pub const TEAM: &str = "AAA";
/// The issues kept unarchived, under the free plan's 250 (linear.app/pricing, Free: « 250 issues »).
pub const ROOM: usize = 240;
/// A closed issue goes to the archive this long after it closed, the running cycle notwithstanding: the cap comes first.
const KEEP_DAYS: i64 = 7;
/// Unanswered writes counted before the sync halts: Linear down answers none, and 285 writes at 60 s each held one run for hours.
const UNANSWERED: usize = 3;
/// The subtitle of the plain link that stands in for an open PR his integration refused: redone once the PR moves.
const UNLINKED: &str = "open, not linked by GitHub";
/// The subtitles the sync writes on a PR's plain link: the board's PR states.
const PR_STATES: [&str; 3] = ["open", "closed", "merged"];

/// An issue as the sync writes it: a task with the column the board computes for it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Issue {
    pub key: String,
    pub title: String,
    pub why: Option<String>,
    pub done_when: Option<String>,
    /// The board's column: in-review, started, todo, backlog, done or canceled.
    pub column: &'static str,
    pub priority: Option<i64>,
    pub estimate: Option<i64>,
    pub epic: Option<String>,
    /// Its roadmap item's project, which takes the place of its epic's.
    pub project: Option<String>,
    /// The set's labels, the agent's among them.
    pub labels: Vec<String>,
    pub parent: Option<String>,
    pub due: Option<String>,
    pub every: Option<String>,
    /// Its tier in the pick order (rfc-rust-unified 20g): its roadmap item's horizon's place in `dash_core::tasks::HORIZONS`;
    /// none on no item, ranked after every horizon, so an issue built by default never jumps the queue (/code-review).
    pub tier: Option<usize>,
    pub claimed: bool,
    pub added: Option<String>,
    pub done: Option<String>,
    pub prs: Vec<LinkedPr>,
}

/// A run's effort, in Fibonacci points: the size its writer judged it before the run.
fn effort_points(effort: &str) -> Option<i64> {
    match effort {
        "low" => Some(1),
        "medium" => Some(2),
        "high" => Some(3),
        "xhigh" => Some(5),
        "max" => Some(8),
        _ => None,
    }
}

/// The board's column, as `Issues.computed/1`: an open PR ready for review is In Review, a claim In Progress, a done task with
/// a closing PR still open In Progress, a dropped one with no claim Canceled.
fn column(t: &LinearTask, claimed: bool) -> &'static str {
    let open: Vec<&LinkedPr> = t.prs.iter().filter(|p| p.state == "open").collect();
    match t.stage.as_str() {
        "dropped" if !claimed => "canceled",
        _ if open.iter().any(|p| !p.draft) => "in-review",
        _ if claimed => "started",
        "done" if open.iter().any(|p| p.kind == "closing") => "started",
        "done" => "done",
        _ if !open.is_empty() => "started",
        "backlog" => "backlog",
        _ => "todo",
    }
}

/// Each task as an issue: `claimed`, the ids Rust's runs hold; `items`, the roadmap, for its project's name.
pub fn issues(tasks: Vec<LinearTask>, claimed: &HashSet<String>, items: &[Item]) -> Vec<Issue> {
    tasks
        .into_iter()
        .map(|t| {
            let claim = t.claimed || claimed.contains(&t.id);
            let column = column(&t, claim);
            let item = (t.item.as_ref()).and_then(|id| items.iter().find(|i| &i.item.id == id));
            let project = item.map(roadmap::project_name);
            let horizons = dash_core::tasks::HORIZONS;
            let tier = item.map(|i| {
                (horizons.iter().position(|h| *h == i.item.horizon)).unwrap_or(horizons.len())
            });
            // who did it: a vibe run is vibe's whatever its owner says; his own work has no label, he is every assignee
            let who = match t.agent.as_deref() {
                Some("vibe") => "vibe",
                _ => t.owner.as_str(),
            };
            let mut labels = t.labels.clone();
            match who {
                "claude" => labels.push("Claude".into()),
                "vibe" => labels.push("vibe".into()),
                _ => {}
            }
            Issue {
                estimate: t.estimate.or(t.effort.as_deref().and_then(effort_points)),
                key: t.id,
                title: t.title,
                why: t.why,
                done_when: t.done_when,
                column,
                priority: t.priority.filter(|p| (1..=4).contains(p)),
                epic: t.epic,
                project,
                labels,
                parent: t.parent,
                due: t.due,
                every: t.every,
                tier,
                claimed: claim,
                added: t.added,
                done: t.done,
                prs: t.prs,
            }
        })
        .collect()
}

fn closed(i: &Issue) -> bool {
    matches!(i.column, "done" | "canceled")
}

/// A canceled task Linear never held: no cycle counts it, so it is never made.
fn skipped(i: &Issue, have: &Have) -> bool {
    i.column == "canceled" && !have.contains_key(&i.key)
}

/// The column Linear's state follows: a closing PR merged and none open moved the issue to Done through his integration, and
/// the task's :done lands later, so the sync never takes it back to Todo meanwhile; a recurring or claimed task is open.
fn state_column(i: &Issue) -> &'static str {
    let merged = i
        .prs
        .iter()
        .any(|p| p.kind == "closing" && p.state == "merged");
    let open = i.prs.iter().any(|p| p.state == "open");
    match i.column {
        "todo" | "started" if merged && !open && i.every.is_none() && !i.claimed => "done",
        c => c,
    }
}

/// The board's column => Linear's workflow state type.
fn state_type_of(column: &str) -> &'static str {
    match column {
        "in-review" | "started" => "started",
        "backlog" => "backlog",
        "done" => "completed",
        "canceled" => "canceled",
        _ => "unstarted",
    }
}

// ---- time, as Linear writes it: « 2026-11-10T08:00:00.000Z »

/// Unix seconds of an ISO time, its offset applied; none when it does not read.
fn secs(iso: &str) -> Option<i64> {
    let day = dash_core::days(iso)?;
    let (t, rest) = (iso.get(10..)?.strip_prefix('T')?, iso.get(19..)?);
    let n = |a: usize, b: usize| t.get(a..b)?.parse::<i64>().ok();
    let at = day * 86_400 + n(0, 2)? * 3600 + n(3, 5)? * 60 + n(6, 8)?;
    let zone = rest.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let off = match zone {
        "Z" | "" => 0,
        z => {
            let sign = if z.starts_with('-') { -1 } else { 1 };
            let h: i64 = z.get(1..3)?.parse().ok()?;
            let m: i64 = z.get(4..6)?.parse().ok()?;
            sign * (h * 3600 + m * 60)
        }
    };
    Some(at - off)
}

/// Whether `at` comes before `start`: a time that does not read keeps the issue (a wrong archive drops it from its cycle).
fn before(at: Option<&str>, start: Option<&str>) -> bool {
    match (at, start) {
        (None, _) => false,
        (Some(_), None) => true,
        (Some(a), Some(s)) => matches!((secs(a), secs(s)), (Some(a), Some(s)) if a < s),
    }
}

/// The day a text opens with at `hour` UTC, never after `now`.
fn at(text: Option<&str>, hour: i64, now: i64) -> Option<i64> {
    let day = dash_core::days(text?)?;
    Some((day * 86_400 + hour * 3600).min(now))
}

/// A done task's completion: its :done's day at 18:00 UTC.
fn completed(i: &Issue, now: i64) -> Option<i64> {
    (i.column == "done").then(|| at(i.done.as_deref(), 18, now))?
}

// ---- what Linear holds

type Have = HashMap<String, Value>;

/// linear.edn's value: each task's Linear identifier and url, by its id.
pub type Links = BTreeMap<String, (String, String)>;

/// The team, its states, labels, projects and the workspace's project statuses, in one request.
const TEAM_Q: &str = "query($key: String!) {
  teams(filter: {key: {eq: $key}}, first: 1) {
    nodes {
      id key cyclesEnabled cycleDuration cycleIssueAutoAssignStarted cycleIssueAutoAssignCompleted
      issueEstimationType
      states(first: 250) { nodes { id type position } }
      labels(first: 250) { nodes { id name retiredAt } }
      projects(first: 250) { nodes { id name description content startDate targetDate status { id } lead { id } } }
    }
  }
  projectStatuses(first: 50) { nodes { id type position } }
}";

const ISSUES_Q: &str = "query($team: ID!, $after: String) {
  issues(filter: {team: {id: {eq: $team}}}, first: 250, after: $after, includeArchived: true) {
    nodes {
      id identifier url title description priority dueDate estimate archivedAt completedAt canceledAt state { id }
      parent { id } cycle { id } project { id } assignee { id } labels { nodes { id } }
      attachments(first: 25) { nodes { id url subtitle sourceType } }
    }
    pageInfo { hasNextPage endCursor }
  }
}";

fn text(v: &Value) -> Option<String> {
    v.as_str().map(String::from)
}

fn nodes(v: &Value) -> &[Value] {
    v["nodes"].as_array().map_or(&[], Vec::as_slice)
}

/// The team, with the workspace's project statuses beside it.
fn team(post: &mut Post) -> Result<Value, String> {
    let data = post(TEAM_Q, json!({"key": TEAM}))?;
    let mut t = (data["teams"]["nodes"].get(0).cloned())
        .ok_or(format!("linear: no team {TEAM} in the workspace"))?;
    t["projectStatuses"] = data["projectStatuses"]["nodes"].clone();
    Ok(t)
}

/// The mark that makes an issue ours, read as plain text, since Linear escapes its punctuation.
fn mark(description: &str) -> Option<String> {
    let plain = plain(description);
    let last = plain.rsplit('\n').next()?;
    let key = last.strip_prefix("site_perso: `")?.strip_suffix('`')?;
    (!key.is_empty() && !key.contains('`')).then(|| key.to_string())
}

/// The team's issues this sync made, by key; and the unarchived ones it did not make, counted against the room.
fn synced(post: &mut Post, team: &str) -> Result<(Have, Vec<Value>), String> {
    let (mut have, mut only) = (Have::new(), vec![]);
    let mut after = Value::Null;
    loop {
        let data = post(ISSUES_Q, json!({"team": team, "after": after}))?;
        let page = &data["issues"];
        let list =
            (page["nodes"].as_array()).ok_or("linear: the issues query answered no issues")?;
        for n in list {
            match mark(n["description"].as_str().unwrap_or_default()) {
                Some(key) => {
                    have.insert(key, n.clone());
                }
                None if n["archivedAt"].is_null() => only.push(n.clone()),
                None => {}
            }
        }
        match (
            page["pageInfo"]["hasNextPage"].as_bool(),
            page["pageInfo"]["endCursor"].as_str(),
        ) {
            (Some(true), Some(c)) => after = json!(c),
            _ => return Ok((have, only)),
        }
    }
}

/// Linear's weekly cycles from Monday, an issue added to the active one when it starts or ends: velocity; and estimates on the
/// Fibonacci scale. True when this run turned them on.
fn cycles(post: &mut Post, team: &Value) -> Result<bool, String> {
    let want = json!({"cyclesEnabled": true, "cycleDuration": 1, "cycleStartDay": 1, "cycleCooldownTime": 0,
        "cycleIssueAutoAssignStarted": true, "cycleIssueAutoAssignCompleted": true, "issueEstimationType": "fibonacci"});
    let checked = [
        "cyclesEnabled",
        "cycleDuration",
        "cycleIssueAutoAssignStarted",
        "cycleIssueAutoAssignCompleted",
        "issueEstimationType",
    ];
    if checked.iter().all(|k| {
        team[*k]
            .as_f64()
            .map_or(team[*k] == want[*k], |n| want[*k].as_f64() == Some(n))
    }) {
        return Ok(false);
    }
    let q = "mutation($id: String!, $in: TeamUpdateInput!) { teamUpdate(id: $id, input: $in) { success } }";
    match post(q, json!({"id": team["id"], "in": want}))?["teamUpdate"]["success"].as_bool() {
        Some(true) => Ok(true),
        _ => Err("linear: the team's cycles were not turned on".into()),
    }
}

/// The running cycle (id, startsAt); with none, the upcoming one starts today, so velocity counts from now, and is read again:
/// else every closed issue was judged with no cycle and the ones done inside it archived.
fn running_cycle(post: &mut Post, team: &str, now: i64) -> Result<Option<Value>, String> {
    // the upcoming ones only: a year of weekly cycles is past a page, and the next one fell off it (/code-review)
    let q = "query($id: String!) { team(id: $id) { activeCycle { id startsAt } \
             cycles(filter: {isFuture: {eq: true}}, first: 10) { nodes { id startsAt } } } }";
    let read = |post: &mut Post| -> Result<Value, String> {
        let data = post(q, json!({"id": team}))?;
        (!data["team"].is_null())
            .then(|| data["team"].clone())
            .ok_or("linear: the team's cycles query answered no team".into())
    };
    let t = read(post)?;
    if !t["activeCycle"].is_null() {
        return Ok(Some(t["activeCycle"].clone()));
    }
    let upcoming = (nodes(&t["cycles"]).iter())
        .filter(|c| {
            c["startsAt"]
                .as_str()
                .and_then(secs)
                .is_some_and(|s| s > now)
        })
        .min_by_key(|c| c["startsAt"].as_str().and_then(secs));
    let Some(upcoming) = upcoming else {
        return Ok(None);
    };
    let q = "mutation($id: String!) { cycleStartUpcomingCycleToday(id: $id) { success } }";
    if post(q, json!({"id": upcoming["id"]}))?["cycleStartUpcomingCycleToday"]["success"] != true {
        return Err("linear: the upcoming cycle did not start".into());
    }
    // the started cycle as read back, else (a dry run's start, sent nowhere) the one started, from now
    let t = read(post)?;
    Ok(Some(match t["activeCycle"].is_null() {
        false => t["activeCycle"].clone(),
        true => json!({"id": upcoming["id"], "startsAt": dash_core::iso(now)}),
    }))
}

// ---- the room under the free plan's cap

/// Its rank among the open: a run at work or in review, then todo, backlog, and last one whose PR merged.
fn rank_group(i: &Issue) -> u8 {
    match state_column(i) {
        "started" | "in-review" => 1,
        "backlog" => 3,
        "done" => 4,
        _ => 2,
    }
}

/// The open issues ranked for the room under the cap: work at work or in review keeps its issue first, then the pick order's keys
/// this list holds (rfc-rust-unified 20g: its tier, its priority, the smaller estimate), so Linear holds the tasks /tasks would
/// start first; then one Linear holds unarchived before one it does not (a new task never pushes out an older one of its rank, an
/// archive and a create at each sync), newest first. A parent ranks with its best open sub-issue, just before it, so a kept
/// sub-issue never loses its parent to the cut. 20g's deadline, impact and unblocks are Rust's store's, not this list's.
fn rank<'a>(open: &[&'a Issue], have: &Have) -> Vec<&'a Issue> {
    let own = |i: &Issue| {
        let held = have.get(&i.key).is_some_and(|h| h["archivedAt"].is_null());
        let group = rank_group(i);
        (
            group,
            i.tier.unwrap_or(dash_core::tasks::HORIZONS.len()),
            i.priority.unwrap_or(5),
            i.estimate.unwrap_or(13),
            u8::from(!held),
        )
    };
    // each one's own rank, then each parent's lowered to its best open sub-issue's: once, never a scan per comparison
    let mut best: HashMap<&str, (u8, usize, i64, i64, u8)> =
        open.iter().map(|i| (i.key.as_str(), own(i))).collect();
    // carried up until nothing moves, so a grandparent ranks with its best grandchild (/code-review); values only fall, so it
    // ends, at most once per level
    let mut moved = true;
    while moved {
        moved = false;
        for k in open {
            let mine = best[k.key.as_str()];
            if let Some(b) = k.parent.as_deref().and_then(|p| best.get_mut(p))
                && mine < *b
            {
                *b = mine;
                moved = true;
            }
        }
    }
    // its depth among the open, so each parent sorts just before its sub-issues at an equal rank; bounded against a cycle
    let parent: HashMap<&str, &str> = (open.iter())
        .filter_map(|i| Some((i.key.as_str(), i.parent.as_deref()?)))
        .filter(|(_, p)| best.contains_key(p))
        .collect();
    let depth = |mut k: &'a str| {
        let mut d = 0;
        while let Some(p) = parent.get(k).filter(|_| d < open.len()) {
            (k, d) = (p, d + 1);
        }
        d
    };
    let mut ranked = open.to_vec();
    ranked.sort_by(|a, b| b.added.cmp(&a.added));
    ranked.sort_by_cached_key(|i| (best[i.key.as_str()], depth(&i.key)));
    ranked
}

/// The open tasks left out, and the closed ones archived, under `ROOM`: whatever else is unarchived (ours of a task gone, those
/// made in Linear) first, then the open by rank, then the newest closed, by the task's :done (Linear's completedAt is wrong for
/// past tasks), one not yet archived before one already archived.
fn room(
    issues: &[Issue],
    have: &Have,
    theirs: usize,
    now: i64,
) -> (HashSet<String>, HashSet<String>) {
    let mine: HashSet<&str> = issues.iter().map(|i| i.key.as_str()).collect();
    let rest = (have.iter())
        .filter(|(k, h)| !mine.contains(k.as_str()) && h["archivedAt"].is_null())
        .count();
    let open: Vec<&Issue> = issues.iter().filter(|i| !closed(i)).collect();
    let ranked = rank(&open, have);
    let cut = ROOM.saturating_sub(rest + theirs).min(ranked.len());
    let waiting = ranked[cut..].iter().map(|i| i.key.clone()).collect();
    let used = cut + rest + theirs;
    let mut shut: Vec<&Issue> = (issues.iter())
        .filter(|i| closed(i) && !skipped(i, have))
        .collect();
    let stamp = |i: &Issue| {
        let h = have.get(&i.key);
        let unarchived = h.is_none_or(|h| h["archivedAt"].is_null());
        // one with no stamp yet is the newest
        let at = completed(i, now)
            .map(dash_core::iso)
            .or_else(|| h.and_then(|h| text(&h["completedAt"]).or_else(|| text(&h["canceledAt"]))));
        (unarchived, at.unwrap_or_else(|| "9".into()))
    };
    shut.sort_by_key(|i| std::cmp::Reverse(stamp(i)));
    let evict = shut
        .iter()
        .skip(ROOM.saturating_sub(used))
        .map(|i| i.key.clone())
        .collect();
    (waiting, evict)
}

// ---- labels and projects

/// The set's labels by name, made once when the team lacks them; one held retired is left off, one Linear refuses too.
fn labels(post: &mut Post, team: &Value) -> HashMap<String, String> {
    let held = nodes(&team["labels"]);
    let live: HashMap<String, String> = (held.iter())
        .filter(|l| l["retiredAt"].is_null())
        .filter_map(|l| Some((text(&l["name"])?.to_lowercase(), text(&l["id"])?)))
        .collect();
    let any: HashSet<String> = (held.iter())
        .filter_map(|l| Some(text(&l["name"])?.to_lowercase()))
        .collect();
    let mut by_name = HashMap::new();
    let q = "mutation($in: IssueLabelCreateInput!) { issueLabelCreate(input: $in) { issueLabel { id } } }";
    for name in dash_store::older::LABELS {
        if let Some(id) = live.get(&name.to_lowercase()) {
            by_name.insert(name.to_string(), id.clone());
        } else if !any.contains(&name.to_lowercase())
            && let Ok(made) = post(q, json!({"in": {"name": name, "teamId": team["id"]}}))
            && let Some(id) = text(&made["issueLabelCreate"]["issueLabel"]["id"])
        {
            by_name.insert(name.to_string(), id);
        }
    }
    by_name
}

fn create_project(
    post: &mut Post,
    team: &Value,
    name: &str,
    mut want: Value,
) -> Result<String, String> {
    want["name"] = json!(name);
    want["teamIds"] = json!([team["id"]]);
    let q = "mutation($in: ProjectCreateInput!) { projectCreate(input: $in) { project { id } } }";
    text(&post(q, json!({"in": want}))?["projectCreate"]["project"]["id"])
        .ok_or(format!("linear: the project {name} was not made"))
}

/// Linear's project status type for an item's status.
fn status_type(status: &str) -> &'static str {
    match status {
        "Completed" => "completed",
        "In Progress" => "started",
        "Planned" => "planned",
        _ => "backlog",
    }
}

/// What a roadmap project holds: its progress as its description (Linear's bar counts only the issues it holds, and the API
/// cannot write it), its horizon, done-when and page as its content, its status, dates and lead.
fn project_fields(i: &Item, statuses: &HashMap<String, String>, me: &str) -> Value {
    let description = match i.progress() {
        Some(p) => format!(
            "{p} % of {} points on /plan, every project's tasks. Linear's bar counts the site_perso ones it holds.",
            i.item.points
        ),
        None => "No points to count yet on /plan.".into(),
    };
    let mut horizon = i.item.horizon.clone();
    if let Some(first) = horizon.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    let content = format!(
        "**{horizon}** on the roadmap.\n\n**Done when**\n\n{}\n\nIts page: http://100.72.158.115:7788/plan?item={}",
        i.item.dod.as_deref().unwrap_or("Not set yet."),
        i.item.id
    );
    let mut want = json!({"description": description, "content": content, "startDate": i.start(),
        "targetDate": i.target(), "leadId": me});
    // a status the workspace lacks is left as Linear holds it, never sent as null
    if let Some(id) = statuses.get(status_type(i.status())) {
        want["statusId"] = json!(id);
    }
    want
}

/// Each project the issues need by name: each roadmap item's, made or set when one of its fields changed, and each epic's made
/// when missing. A roadmap project Linear refuses to update keeps its issues: said in `notes`, never a halt. Projects made.
fn projects(
    post: &mut Post,
    team: &Value,
    items: &[Item],
    epics: &[&str],
    me: &str,
    notes: &mut Vec<String>,
) -> Result<(HashMap<String, String>, usize), String> {
    let held: HashMap<String, &Value> = (nodes(&team["projects"]).iter())
        .filter_map(|p| Some((text(&p["name"])?, p)))
        .collect();
    let mut statuses: Vec<&Value> = team["projectStatuses"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    statuses.sort_by(|a, b| {
        b["position"]
            .as_f64()
            .partial_cmp(&a["position"].as_f64())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let statuses: HashMap<String, String> = statuses
        .iter()
        .filter_map(|s| Some((text(&s["type"])?, text(&s["id"])?)))
        .collect();
    let mut ids: HashMap<String, String> = held
        .iter()
        .filter_map(|(n, p)| Some((n.clone(), text(&p["id"])?)))
        .collect();
    let mut made = 0;
    for i in items {
        let name = roadmap::project_name(i);
        let want = project_fields(i, &statuses, me);
        match held.get(&name) {
            Some(old) => {
                let now = json!({"description": old["description"], "content": plain_value(&old["content"]),
                    "startDate": old["startDate"], "targetDate": old["targetDate"], "leadId": old["lead"]["id"],
                    "statusId": old["status"]["id"]});
                let differs = (want.as_object().into_iter().flatten()).any(|(k, v)| {
                    if k == "content" {
                        plain_value(v) != now[k]
                    } else {
                        *v != now[k]
                    }
                });
                if differs {
                    let q = "mutation($id: String!, $in: ProjectUpdateInput!) { projectUpdate(id: $id, input: $in) { success } }";
                    let done = post(q, json!({"id": old["id"], "in": want}));
                    if !matches!(&done, Ok(d) if d["projectUpdate"]["success"] == true) {
                        notes.push(format!(
                            "{name} kept as it was: {}",
                            done.err().unwrap_or("not updated".into())
                        ));
                    }
                }
            }
            None => {
                // a create refused on a timeline field is made with its text alone: the next sync's update sends the rest
                let id = create_project(post, team, &name, want.clone()).or_else(|why| {
                    notes.push(format!("{name} made without its timeline: {why}"));
                    let text =
                        json!({"description": want["description"], "content": want["content"]});
                    create_project(post, team, &name, text)
                });
                // a refusal is a note, never a halt: its issues go without a project this run (/code-review)
                match id {
                    Ok(id) => {
                        ids.insert(name, id);
                        made += 1;
                    }
                    Err(e) if unanswered(&e) => return Err(e),
                    Err(e) => notes.push(format!("{name} not made: {e}")),
                }
            }
        }
    }
    for epic in epics {
        if ids.contains_key(*epic) {
            continue;
        }
        let desc =
            format!("The site_perso epic « {epic} », its tasks synced by `dash linear sync`.");
        match create_project(post, team, epic, json!({"description": desc})) {
            Ok(id) => {
                ids.insert(epic.to_string(), id);
                made += 1;
            }
            Err(e) if unanswered(&e) => return Err(e),
            Err(e) => notes.push(format!("{epic} not made: {e}")),
        }
    }
    Ok((ids, made))
}

// ---- one issue

/// What one run knows: the team, its labels and projects by name, his user, the running cycle and the room.
struct Refs {
    team: Value,
    labels: HashMap<String, String>,
    projects: HashMap<String, String>,
    me: String,
    cycle: Option<Value>,
    waiting: HashSet<String>,
    evict: HashSet<String>,
    now: i64,
}

impl Refs {
    /// The team's first state of that type, as Linear's board puts a new issue there.
    fn state(&self, kind: &str) -> Option<String> {
        (nodes(&self.team["states"]).iter())
            .filter(|s| s["type"] == kind)
            .min_by(|a, b| {
                a["position"]
                    .as_f64()
                    .partial_cmp(&b["position"].as_f64())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .and_then(|s| text(&s["id"]))
    }

    fn state_type(&self, id: &str) -> Option<String> {
        (nodes(&self.team["states"]).iter())
            .find(|s| s["id"] == id)
            .and_then(|s| text(&s["type"]))
    }
}

/// Its description: why, then its done-when, then the mark.
fn description(i: &Issue) -> String {
    let parts: Vec<String> = [
        i.why.clone(),
        i.done_when
            .as_ref()
            .map(|d| format!("**Done when**\n\n{d}")),
    ]
    .into_iter()
    .flatten()
    .filter(|p| !p.is_empty())
    .collect();
    let text = format!("{}\n\nsite_perso: `{}`", parts.join("\n\n"), i.key);
    text.trim_start().to_string()
}

/// What Linear should hold for this issue.
fn fields(r: &Refs, have: &Have, i: &Issue) -> Map<String, Value> {
    let state = r
        .state(state_type_of(state_column(i)))
        .or_else(|| r.state("unstarted"));
    let project = (i.project.as_ref().and_then(|p| r.projects.get(p)))
        .or_else(|| i.epic.as_ref().and_then(|e| r.projects.get(e)));
    let mut seen = HashSet::new();
    let labels: Vec<String> = (i.labels.iter())
        .filter_map(|l| r.labels.get(l).cloned())
        .filter(|id| seen.insert(id.clone()))
        .collect();
    let mut f = Map::new();
    f.insert("title".into(), json!(i.title));
    f.insert("description".into(), json!(description(i)));
    f.insert("stateId".into(), json!(state));
    f.insert("priority".into(), json!(i.priority.unwrap_or(0)));
    f.insert("assigneeId".into(), json!(r.me));
    f.insert("projectId".into(), json!(project));
    f.insert("labelIds".into(), json!(labels));
    // sent only when the task has one, so one he set in Linear is kept
    if let Some(e) = i.estimate {
        f.insert("estimate".into(), json!(e));
    }
    // done inside the running cycle by its :done: in that cycle, as Linear adds an issue completed while it runs
    if let Some(c) = &r.cycle
        && let Some(at) = completed(i, r.now)
        && !before(Some(&dash_core::iso(at)), c["startsAt"].as_str())
    {
        f.insert("cycleId".into(), c["id"].clone());
    }
    if let Some(parent) = i
        .parent
        .as_ref()
        .and_then(|p| have.get(p))
        .and_then(|h| text(&h["id"]))
    {
        f.insert("parentId".into(), json!(parent));
    }
    // Linear's recurring issues cannot be set through its API: a recurring task shows the day it opens again as its due date,
    // cleared once it opens; another task's due date is sent only when it holds one, so one he set in Linear stays
    if i.every.is_some() {
        f.insert("dueDate".into(), json!(i.due));
    } else if let Some(d) = &i.due {
        f.insert("dueDate".into(), json!(d));
    }
    f
}

/// Whether Linear holds `want` already, in the fields `fields` writes: descriptions read as Linear rewrites them.
fn same(old: &Value, want: &Map<String, Value>) -> bool {
    let id = |v: &Value| v["id"].clone();
    let mut labels: Vec<Value> = nodes(&old["labels"]).iter().map(id).collect();
    labels.sort_by_key(|v| v.to_string());
    let held = json!({"title": old["title"], "description": plain_value(&old["description"]),
        "stateId": old["state"]["id"], "priority": old["priority"], "dueDate": old["dueDate"],
        "estimate": old["estimate"], "labelIds": labels, "projectId": old["project"]["id"],
        "assigneeId": old["assignee"]["id"], "parentId": old["parent"]["id"], "cycleId": old["cycle"]["id"]});
    want.iter().all(|(k, v)| {
        let v = match k.as_str() {
            "description" => plain_value(v),
            "labelIds" => {
                let mut l = v.as_array().cloned().unwrap_or_default();
                l.sort_by_key(|v| v.to_string());
                json!(l)
            }
            _ => v.clone(),
        };
        match (v.as_f64(), held[k].as_f64()) {
            (Some(a), Some(b)) => a == b,
            _ => v == held[k],
        }
    })
}

/// How a write ended, counted.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Did {
    Created,
    Updated,
    Same,
    Archived,
    Closed,
}

/// The issue written: created, updated or the same, its node (id, identifier, url) passed on.
fn upsert(post: &mut Post, r: &Refs, have: &Have, i: &Issue) -> Result<(Did, Value), String> {
    let old = have.get(&i.key);
    let mut want = fields(r, have, i);
    // a state Linear holds of the type the board wants stays: his GitHub integration moves an issue to In Review, another
    // started state, and the sync wrote In Progress back at each run
    if let Some(held) = old.and_then(|o| o["state"]["id"].as_str())
        && r.state_type(held) == want["stateId"].as_str().and_then(|w| r.state_type(w))
    {
        want.insert("stateId".into(), json!(held));
    }
    let Some(old) = old else {
        want.insert("teamId".into(), r.team["id"].clone());
        // when it was made and done, as the task says (Linear takes both on create only, each in the past, done after made)
        let done = completed(i, r.now - 60);
        let made = at(i.added.as_deref(), 8, r.now - 180).or(done.map(|d| d - 3600));
        if let Some(m) = made {
            want.insert("createdAt".into(), json!(dash_core::iso(m)));
        }
        if let Some(d) = done {
            want.insert(
                "completedAt".into(),
                json!(dash_core::iso(d.max(made.unwrap_or(d) + 60))),
            );
        }
        let q = "mutation($in: IssueCreateInput!) { issueCreate(input: $in) { issue { id identifier url } } }";
        let node = post(q, json!({"in": want}))?["issueCreate"]["issue"].clone();
        return match node["id"].is_string() {
            true => Ok((Did::Created, node)),
            false => Err("linear: issueCreate answered no issue".into()),
        };
    };
    if same(old, &want) {
        return Ok((Did::Same, old.clone()));
    }
    let q = "mutation($id: String!, $in: IssueUpdateInput!) { issueUpdate(id: $id, input: $in) { issue { id identifier url } } }";
    let node = post(q, json!({"id": old["id"], "in": want}))?["issueUpdate"]["issue"].clone();
    match node["id"].is_string() {
        true => Ok((Did::Updated, node)),
        false => Err("linear: issueUpdate answered no issue".into()),
    }
}

/// `issueArchive` or `issueUnarchive` of the issue `node` names: the node with its archivedAt as this run left it.
fn archive(post: &mut Post, node: &Value, field: &str, now: i64) -> Result<Value, String> {
    let q = format!("mutation($id: String!) {{ {field}(id: $id) {{ success }} }}");
    if post(&q, json!({"id": node["id"]}))?[field]["success"] != true {
        return Err(format!("linear: {field} did not succeed"));
    }
    let mut node = node.clone();
    node["archivedAt"] = match field {
        "issueArchive" => json!(dash_core::iso(now)),
        _ => Value::Null,
    };
    Ok(node)
}

/// One issue: skipped (a canceled task Linear never held), archived (closed before the running cycle began, or past
/// `KEEP_DAYS`, or with no room), or kept, taken out of the archive first when open again.
fn put(post: &mut Post, r: &Refs, have: &Have, i: &Issue) -> Result<(Did, Option<Value>), String> {
    let old = have.get(&i.key);
    let over = || {
        let Some(c) = &r.cycle else { return true };
        let at = completed(i, r.now).map(dash_core::iso).or_else(|| {
            old.and_then(|o| text(&o["completedAt"]).or_else(|| text(&o["canceledAt"])))
        });
        let week = dash_core::iso(r.now - KEEP_DAYS * 86_400);
        before(at.as_deref(), c["startsAt"].as_str()) || before(at.as_deref(), Some(&week))
    };
    if skipped(i, have) && !r.waiting.contains(&i.key) {
        return Ok((Did::Closed, None));
    }
    let archive_it = r.waiting.contains(&i.key)
        || (closed(i) && !skipped(i, have) && (over() || r.evict.contains(&i.key)));
    let archived = old.is_some_and(|o| !o["archivedAt"].is_null());
    match (archive_it, archived) {
        (true, true) | (false, false) => upsert(post, r, have, i).map(|(d, n)| (d, Some(n))),
        (true, false) => {
            let (_, node) = upsert(post, r, have, i)?;
            Ok((
                Did::Archived,
                Some(archive(post, &node, "issueArchive", r.now)?),
            ))
        }
        (false, true) => {
            archive(post, old.unwrap_or(&Value::Null), "issueUnarchive", r.now)?;
            let (_, mut node) = upsert(post, r, have, i)?;
            node["archivedAt"] = Value::Null;
            Ok((Did::Updated, Some(node)))
        }
    }
}

fn full(e: &str) -> bool {
    e.starts_with("linear: usage limit exceeded")
}

/// A call Linear left unanswered (`roadmap::linear_post`'s transport failure).
fn unanswered(e: &str) -> bool {
    e.starts_with("linear: the API failed on")
}

/// What one sync did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Counts {
    pub created: usize,
    pub updated: usize,
    pub same: usize,
    pub archived: usize,
    /// canceled and never synced, left out
    pub closed: usize,
    /// a create the plan refused
    pub refused: usize,
    /// a write Linear left unanswered
    pub failed: usize,
    pub attached: usize,
    pub projects: usize,
    pub cycles: bool,
    pub only: usize,
    pub waiting: usize,
}

/// Every issue written, the closed and the open with no room that Linear holds first, so their archives free the room before a
/// create needs it; a sub-issue after its parent, so the parent's id is known. The issues as now written come back in `have`,
/// even when one halts the run.
fn put_all(
    post: &mut Post,
    r: &Refs,
    have: &mut Have,
    issues: &[&Issue],
    n: &mut Counts,
) -> Result<(), String> {
    let mut order: Vec<&&Issue> = issues.iter().collect();
    order.sort_by_key(|i| {
        let frees = (closed(i) || r.waiting.contains(&i.key)) && have.contains_key(&i.key);
        (!frees, i.parent.is_some())
    });
    for i in order {
        match put(post, r, have, i) {
            Ok((did, node)) => {
                if let Some(node) = node {
                    // the written node's identifier and url over the ones read before: a rename changes the url
                    let entry = have.entry(i.key.clone()).or_insert(json!({}));
                    for (k, v) in node.as_object().into_iter().flatten() {
                        entry[k] = v.clone();
                    }
                }
                *match did {
                    Did::Created => &mut n.created,
                    Did::Updated => &mut n.updated,
                    Did::Same => &mut n.same,
                    Did::Archived => &mut n.archived,
                    Did::Closed => &mut n.closed,
                } += 1;
            }
            // one write unanswered is counted and tried again next run, never a halt of the hundreds after it
            Err(e) if unanswered(&e) && n.failed < UNANSWERED => n.failed += 1,
            // the free plan makes no issue while 250 are open: counted, and the updates, archives and PRs go on
            Err(e) if full(&e) => n.refused += 1,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

// ---- PRs

fn closing(pr: &LinkedPr) -> bool {
    pr.kind == "closing"
}

/// Whether a PR is due on its issue, given the attachment held at its address: the integration's own keeps its status live; a
/// plain link of the sync's is sent again once the PR moved; one with another subtitle is someone else's.
fn due(pr: &LinkedPr, held: Option<&Value>) -> bool {
    let Some(h) = held else { return true };
    let sub = h["subtitle"].as_str();
    if h["sourceType"] == "github" {
        return false;
    }
    if sub == Some(UNLINKED) {
        return pr.state != "open";
    }
    (sub.is_none() || sub.is_some_and(|s| PR_STATES.contains(&s)))
        && ((pr.state == "open" && closing(pr)) || sub != Some(pr.state.as_str()))
}

fn plain_link(post: &mut Post, id: &str, pr: &LinkedPr, subtitle: &str) -> Result<(), String> {
    let q = "mutation($in: AttachmentCreateInput!) { attachmentCreate(input: $in) { success } }";
    let title = pr
        .number
        .map_or("Pull request".into(), |n| format!("Pull request #{n}"));
    let input = json!({"issueId": id, "url": pr.url, "title": title, "subtitle": subtitle});
    match post(q, json!({"in": input}))?["attachmentCreate"]["success"] == true {
        true => Ok(()),
        false => Err(format!("linear: {} was not attached", pr.url)),
    }
}

/// An open closing PR through his GitHub integration, which refuses a second attachment on one address, so the held plain link
/// goes first; one it refuses gets a plain link standing in. A merged or closed one as a plain link.
fn link(post: &mut Post, id: &str, pr: &LinkedPr, held: Option<&Value>) -> Result<(), String> {
    if !(pr.state == "open" && closing(pr)) {
        return plain_link(post, id, pr, &pr.state);
    }
    if let Some(h) = held {
        let q = "mutation($id: String!) { attachmentDelete(id: $id) { success } }";
        match post(q, json!({"id": h["id"]})) {
            Ok(d) if d["attachmentDelete"]["success"] == true => {}
            // gone already: nothing left to delete (/code-review)
            Err(e) if e.starts_with("linear: Entity not found: Attachment") => {}
            Err(e) => return Err(e),
            Ok(_) => return Err(format!("linear: {} was not attached", pr.url)),
        }
    }
    let q = "mutation($id: String!, $url: String!) { attachmentLinkGitHubPR(issueId: $id, url: $url, linkKind: closes) { success } }";
    match post(q, json!({"id": id, "url": pr.url})) {
        Ok(d) if d["attachmentLinkGitHubPR"]["success"] == true => Ok(()),
        // an archived issue is taken out by `attach`; a call left unanswered is tried again next run, never stood in for by a
        // plain link the next runs keep while the PR is open (/code-review)
        Err(e) if e.starts_with("linear: Entity not found") || unanswered(&e) => Err(e),
        // the integration refused it (a repo it does not see): a plain link stands in, redone once the PR moves
        _ => plain_link(post, id, pr, UNLINKED),
    }
}

/// Each PR its issue lacks or holds out of date, attached; an archived issue (« Entity not found ») is taken out of the archive
/// for it, then put back whatever the attachment answered. Once the plan refuses one unarchive, the other archived issues wait
/// for the next run. How many were attached, and each PR Linear refused, said: one refusal never halts the sync before the
/// roadmap's milestones and waits (/code-review).
fn attach(post: &mut Post, have: &Have, issues: &[Issue], now: i64) -> (usize, Vec<String>) {
    let mut rows = vec![];
    for i in issues {
        let Some(old) = have.get(&i.key) else {
            continue;
        };
        let Some(id) = old["id"].as_str() else {
            continue;
        };
        let held: HashMap<&str, &Value> = nodes(&old["attachments"])
            .iter()
            .filter_map(|a| Some((a["url"].as_str()?, a)))
            .collect();
        for pr in &i.prs {
            let h = held.get(pr.url.as_str()).copied();
            if due(pr, h) {
                rows.push((id.to_string(), pr, h, !old["archivedAt"].is_null()));
            }
        }
    }
    // one call per issue and address, its closing row kept so the integration links it
    rows.sort_by_key(|r| !closing(r.1));
    let mut seen = HashSet::new();
    rows.retain(|r| seen.insert((r.0.clone(), r.1.url.clone())));
    let (mut n, mut no_room, mut refused) = (0, false, vec![]);
    for (id, pr, held, archived) in rows {
        if archived && no_room {
            continue;
        }
        let node = json!({"id": id});
        let done = match link(post, &id, pr, held) {
            Err(e) if e.starts_with("linear: Entity not found: Issue") => {
                archive(post, &node, "issueUnarchive", now).and_then(|_| {
                    let linked = link(post, &id, pr, None);
                    // the link stands even when the archive back fails: said, and the next sync archives it (/code-review)
                    if let Err(e) = archive(post, &node, "issueArchive", now) {
                        refused.push(format!("{} left unarchived after its link: {e}", pr.url));
                    }
                    linked
                })
            }
            done => done,
        };
        match done {
            Ok(()) => n += 1,
            Err(e) if unanswered(&e) => {}
            Err(e) if full(&e) => no_room = true,
            Err(e) => refused.push(format!("{} not attached: {e}", pr.url)),
        }
    }
    (n, refused)
}

// ---- the whole sync

/// What linear.edn should hold: each of this sync's issues Linear holds, its identifier and url, and the older module's example
/// issues, which it keeps there (/code-review: left out, the two writers rewrote linear.edn at each run).
fn links(have: &Have, issues: &[Issue]) -> Links {
    let examples = have.keys().filter(|k| k.starts_with("example-"));
    (issues.iter().map(|i| &i.key).chain(examples))
        .filter_map(|key| {
            let h = have.get(key)?;
            let url = text(&h["url"]).filter(|u| u.starts_with("https://linear.app/"))?;
            Some((
                key.clone(),
                (text(&h["identifier"]).unwrap_or_default(), url),
            ))
        })
        .collect()
}

/// linear.edn's text, as `dash store text linear.edn` prints it.
pub fn links_text(links: &Links) -> String {
    let entries: Vec<Value> = (links.iter())
        .map(|(k, (idf, url))| json!([k, {"identifier": idf, "url": url}]))
        .collect();
    serde_json::to_string_pretty(&json!({"~#map": entries})).unwrap_or_default()
}

/// One sync: `issues` and the roadmap `items` written to Linear through `post`, at Unix second `now`. `save` gets linear.edn's
/// links once the issues are written, even when one halted the run, so the issues written before keep theirs; the milestones
/// and dependencies of each item's plan follow (`roadmap::write_linear`). The counts, and the lines worth saying (a project
/// kept as it was, what the roadmap write did).
pub fn sync(
    issues: &[Issue],
    items: &[Item],
    post: &mut Post,
    now: i64,
    save: &mut dyn FnMut(&Links) -> Result<(), String>,
) -> Result<(Counts, Vec<String>), String> {
    let team = team(post)?;
    let team_id = text(&team["id"]).unwrap_or_default();
    // his key's user, every issue's assignee; read before any write, so a failed read leaves Linear as it was
    let me = text(&post("query { viewer { id } }", json!({}))?["viewer"]["id"])
        .ok_or("linear: the users query answered no viewer")?;
    let mut n = Counts {
        cycles: cycles(post, &team)?,
        ..Counts::default()
    };
    let cycle = running_cycle(post, &team_id, now)?;
    let (mut have, only) = synced(post, &team_id)?;
    let (waiting, evict) = room(issues, &have, only.len(), now);
    // an open task with no room that Linear never held is not made, nor its epic's project
    let new_waiting = |i: &Issue| waiting.contains(&i.key) && !have.contains_key(&i.key);
    let put: Vec<&Issue> = issues.iter().filter(|i| !new_waiting(i)).collect();
    let mut epics: Vec<&str> = (put.iter())
        .filter(|i| !skipped(i, &have) && i.project.is_none())
        .filter_map(|i| i.epic.as_deref())
        .collect();
    epics.sort_unstable();
    epics.dedup();
    let labels = labels(post, &team);
    let mut notes = vec![];
    let (projects, made) = projects(post, &team, items, &epics, &me, &mut notes)?;
    n.projects = made;
    n.only = only.len();
    n.waiting = waiting.len();
    let refs = Refs {
        team,
        labels,
        projects,
        me,
        cycle,
        waiting,
        evict,
        now,
    };
    let written = put_all(post, &refs, &mut have, &put, &mut n);
    let saved = save(&links(&have, issues));
    written?;
    saved?;
    let (attached, refused) = attach(post, &have, issues, now);
    n.attached = attached;
    notes.extend(refused);
    notes.extend(roadmap::write_linear(items, post)?);
    Ok((n, notes))
}

/// The older app's `Dashboard.Linear.Every` still runs, every 30 minutes: it writes the issues and the roadmap projects' status,
/// dates and texts. While it does, a sync writes only the fields Rust alone holds, the milestones and waits, so each field has
/// one writer (the PM check of task rust-linear-sync-port: « one writer per field »). Task rust-older-jobs-stop stops that tick
/// and sets this false in the same change; a dry run reads the whole sync whatever it says.
pub const OLDER_TICK: bool = true;

/// One run: the whole `sync` once Rust is Linear's one writer (`older_tick` false); while the older tick runs, only the fields
/// Rust alone writes, each roadmap item's milestones and waits (`roadmap::write_linear`), and a line saying so. Its lines.
pub fn run(
    issues: &[Issue],
    items: &[Item],
    post: &mut Post,
    now: i64,
    save: &mut dyn FnMut(&Links) -> Result<(), String>,
    older_tick: bool,
) -> Result<Vec<String>, String> {
    if older_tick {
        let head = "linear: the older app's tick writes the issues and the roadmap projects until task rust-older-jobs-stop \
                    stops it; Rust wrote the milestones and waits only (dash linear sync --dry-run reads the whole sync)";
        return Ok(std::iter::once(head.to_string())
            .chain(roadmap::write_linear(items, post)?)
            .collect());
    }
    let (n, notes) = sync(issues, items, post, now, save)?;
    Ok(std::iter::once(said(&n)).chain(notes).collect())
}

/// The counts in words, as the older app's `dash linear sync` said them.
pub fn said(n: &Counts) -> String {
    format!(
        "linear: {} created, {} updated, {} unchanged, {} archived, {} canceled and never synced, {} refused by Linear's plan \
         limit, {} left unanswered; {} PRs attached, {} projects made{}; team {TEAM}: {} made in Linear only, {} open tasks \
         waiting for room",
        n.created,
        n.updated,
        n.same,
        n.archived,
        n.closed,
        n.refused,
        n.failed,
        n.attached,
        n.projects,
        if n.cycles {
            ", weekly cycles turned on"
        } else {
            ""
        },
        n.only,
        n.waiting
    )
}

/// A dry run's Linear: each read asked of `real`, each write answered here with Linear's success shape and counted by its field,
/// so `dash linear sync --dry-run` says what a sync would write and writes nothing.
fn dry<'a>(
    real: &'a mut Post<'a>,
    wrote: &'a mut BTreeMap<String, usize>,
) -> impl FnMut(&str, Value) -> Result<Value, String> + 'a {
    let mut n = 0;
    move |q: &str, vars: Value| {
        if !q.trim_start().starts_with("mutation") {
            return real(q, vars);
        }
        let field = q.split_once('{').map_or("", |(_, b)| b).trim_start();
        let field = field
            .split(['(', ' '])
            .next()
            .unwrap_or_default()
            .to_string();
        *wrote.entry(field.clone()).or_default() += 1;
        n += 1;
        let id = json!(format!("dry-{n}"));
        let node = json!({"id": vars["id"].as_str().map_or(id.clone(), |i| json!(i)), "identifier": "dry",
            "url": "https://linear.app/dry"});
        Ok(match field.as_str() {
            "issueCreate" | "issueUpdate" => json!({field: {"issue": node}}),
            "projectCreate" => json!({field: {"project": {"id": id}}}),
            "issueLabelCreate" => json!({field: {"issueLabel": {"id": id}}}),
            _ => json!({field: {"success": true}}),
        })
    }
}

/// `dash linear sync` on the store at `dir` (Rust's runs and plans) and the older app's list: what it did, a line each. linear.edn
/// is written through the older app only when its links changed: a write rebuilds that app's model. `dry_run`: Linear read, each
/// write counted and none sent, linear.edn left.
pub fn command(dir: &std::path::Path, dry_run: bool) -> Result<String, String> {
    let db = crate::tasks::older_db();
    let items = roadmap::items(&db, &dash_store::tasks::read_plans(dir)?)?;
    // while the older tick writes the issues, they are never read: `run` writes the items' own fields alone (/code-review)
    if OLDER_TICK && !dry_run {
        let mut post = roadmap::linear_post()?;
        let mut save = |_: &Links| Ok(());
        return Ok(run(&[], &items, &mut post, 0, &mut save, true)?.join("\n"));
    }
    let claimed: HashSet<String> = (crate::tasks::open_tasks(dir)?.rows()?.into_iter())
        .filter(|r| r.status == "claimed")
        .map(|r| r.id)
        .collect();
    let issues = issues(dash_store::older::linear_tasks(&db)?, &claimed, &items);
    let held = dash_store::older::linear_links(&db)?;
    // read again before the write: a sync that saved since holds newer links, which stay, and this one writes nothing, as the
    // older module's `base` held them while its tick still runs beside this (/code-review)
    let mut save =
        |links: &Links| match *links == held || dash_store::older::linear_links(&db)? != held {
            true => Ok(()),
            false => crate::tasks::older_app()
                .store_put("linear.edn", &links_text(links))
                .map(drop),
        };
    let mut post = roadmap::linear_post()?;
    let now = crate::tasks::now_ms() / 1000;
    if dry_run {
        let mut wrote = BTreeMap::new();
        let (n, notes) = {
            let mut real: &mut Post = &mut post;
            let mut post = dry(&mut real, &mut wrote);
            let mut save = |_: &Links| Ok(());
            sync(&issues, &items, &mut post, now, &mut save)?
        };
        let writes: Vec<String> = wrote.iter().map(|(f, n)| format!("{f} {n}")).collect();
        let writes = if writes.is_empty() {
            "none".into()
        } else {
            writes.join(", ")
        };
        let head = format!(
            "dry run, nothing sent: {}\nwrites it would send: {writes}",
            said(&n)
        );
        return Ok(std::iter::once(head)
            .chain(notes)
            .collect::<Vec<_>>()
            .join("\n"));
    }
    Ok(run(&issues, &items, &mut post, now, &mut save, OLDER_TICK)?.join("\n"))
}

// ---- Linear's rewrites of what it stores

static ANGLED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([^\]]*)\]\(<[^>]*>\)").expect("regex"));
static ESCAPED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\([[:punct:]])").expect("regex"));
static LINKED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([^\]]*)\]\(([^)]*)\)").expect("regex"));
static SCHEME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^[a-z][a-z0-9+.-]*:").expect("regex"));
static TRAILING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)[ \t]+$").expect("regex"));
static BULLET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^(\s*)- ").expect("regex"));

fn word(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// « _x_ » as Linear stores it, « *x* »: an underscore after no word character or star, its text with no underscore or new
/// line, closed by one followed by no word character.
fn starred(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let prev = i.checked_sub(1).map(|p| chars[p]);
        if chars[i] == '_' && !word(prev) && prev != Some('*') {
            let close = (i + 1..chars.len()).find(|&j| chars[j] == '_' || chars[j] == '\n');
            if let Some(j) = close
                && chars[j] == '_'
                && j > i + 1
                && !word(chars.get(j + 1).copied())
            {
                out.push('*');
                out.extend(&chars[i + 1..j]);
                out.push('*');
                i = j + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// A text as Linear writes back what it stores: a bare address it links (« [x](<http://x>) »), punctuation it escapes, a link
/// whose address is not one losing it, « _x_ » turned « *x* », a « - » bullet « * », spaces ending a line, a trailing new line. Compared this way on
/// both sides, else 24 issues were sent again at each sync for nothing (task linear-sync-drift, 2026-11-02).
pub fn plain(text: &str) -> String {
    let t = ANGLED.replace_all(text, "$1");
    let t = ESCAPED.replace_all(&t, "$1");
    let t = LINKED.replace_all(&t, |c: &regex::Captures| match SCHEME.is_match(&c[2]) {
        true => c[0].to_string(),
        false => c[1].to_string(),
    });
    let t = starred(&t);
    let t = BULLET.replace_all(&t, "$1* ");
    TRAILING.replace_all(&t, "").trim_end().to_string()
}

fn plain_value(v: &Value) -> Value {
    v.as_str().map_or(Value::Null, |s| json!(plain(s)))
}

#[cfg(test)]
mod tests;
