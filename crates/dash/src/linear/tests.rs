use super::*;
use dash_meter::dash_test;
use dash_store::older::RoadmapItem;
use dash_store::tasks::Plan;

/// Linear's answers recorded from team AAA on 2026-11-12, read-only queries, trimmed (tests/fixtures/linear/recorded.json).
const RECORDED: &str = include_str!("../../tests/fixtures/linear/recorded.json");

/// 2026-11-12T12:00:00Z
fn noon() -> i64 {
    dash_core::days("2026-11-12").unwrap() * 86_400 + 12 * 3600
}

/// Linear as recorded, answering each read from the recording and each write with Linear's own success shape, the projects,
/// milestones and relations it makes read back after. `calls`: each write's field and variables, in order.
struct Linear {
    recorded: Value,
    calls: Vec<(String, Value)>,
    made: Vec<Value>,
    n: usize,
}

impl Linear {
    fn new() -> Linear {
        Linear {
            recorded: serde_json::from_str(RECORDED).expect("recorded answers"),
            calls: vec![],
            made: vec![],
            n: 0,
        }
    }

    fn answer(&mut self, q: &str, vars: &Value) -> Result<Value, String> {
        let r = &self.recorded;
        let read = |k: &str| Ok(r[k].clone());
        if !q.trim_start().starts_with("mutation") {
            return match q {
                _ if q.contains("viewer") => read("viewer"),
                _ if q.contains("teams(") => read("team"),
                _ if q.contains("activeCycle") => read("cycle"),
                _ if q.contains("issues(filter") => read("issues"),
                _ if q.contains("projectMilestones") => {
                    let mut p = r["projects"].clone();
                    let nodes = p["projects"]["nodes"].as_array_mut().expect("nodes");
                    nodes.extend(self.made.iter().cloned());
                    for n in nodes.iter_mut() {
                        let id = n["id"].clone();
                        let calls = self.calls.iter();
                        let ms: Vec<Value> = (calls.clone())
                            .filter(|(f, v)| f == "projectMilestoneCreate" && v["in"]["projectId"] == id)
                            .map(|(_, v)| json!({"id": "m", "name": v["in"]["name"], "targetDate": v["in"]["targetDate"]}))
                            .collect();
                        let rs: Vec<Value> = calls
                            .filter(|(f, v)| f == "projectRelationCreate" && v["in"]["projectId"] == id)
                            .map(|(_, v)| json!({"anchorType": "end", "relatedAnchorType": "start",
                                "project": {"id": id}, "relatedProject": {"id": v["in"]["relatedProjectId"]}}))
                            .collect();
                        n["projectMilestones"]["nodes"] = json!(ms);
                        n["relations"]["nodes"] = json!(rs);
                    }
                    Ok(p)
                }
                _ => Err(format!("not recorded: {q}")),
            };
        }
        let field = q.split_once('{').map(|(_, b)| b).unwrap_or_default();
        let field = field
            .trim_start()
            .split(['(', ' '])
            .next()
            .unwrap_or_default()
            .to_string();
        self.calls.push((field.clone(), vars.clone()));
        self.n += 1;
        let issue = |id: Value| {
            json!({"issue": {"id": id, "identifier": format!("AAA-9{:05}", 0),
            "url": "https://linear.app/gdw/issue/AAA-900000/made"}})
        };
        Ok(match field.as_str() {
            "issueCreate" => json!({"issueCreate": issue(json!(format!("new-{}", self.n)))}),
            "issueUpdate" => {
                json!({"issueUpdate": {"issue": {"id": vars["id"], "identifier": "AAA-1", "url": "https://linear.app/gdw/issue/AAA-1/x"}}})
            }
            "projectCreate" => {
                let id = format!("project-{}", self.n);
                self.made
                    .push(json!({"id": id, "name": vars["in"]["name"]}));
                json!({"projectCreate": {"project": {"id": id}}})
            }
            f => json!({f: {"success": true}}),
        })
    }
}

fn node<'a>(have: &'a Value, identifier: &str) -> &'a Value {
    let nodes = have["issues"]["issues"]["nodes"]
        .as_array()
        .expect("issues");
    nodes
        .iter()
        .find(|n| n["identifier"] == identifier)
        .expect("a recorded issue")
}

/// Its why and done-when as the recorded description holds them, so the issue reads the same.
fn said_of(n: &Value) -> (Option<String>, Option<String>) {
    let d = n["description"].as_str().unwrap_or_default();
    let body = d.rsplit_once("\n\nsite_perso:").map_or(d, |(b, _)| b);
    match body.split_once("\n\n**Done when**\n\n") {
        Some((why, dw)) => (Some(why.into()), Some(dw.into())),
        None => (Some(body.into()), None),
    }
}

fn pr(n: i64, state: &str) -> LinkedPr {
    LinkedPr {
        url: format!("https://github.com/nha-org/site_perso/pull/{n}"),
        number: Some(n),
        state: state.into(),
        kind: "closing".into(),
        draft: false,
    }
}

fn issue(key: &str, column: &'static str) -> Issue {
    Issue {
        key: key.into(),
        title: key.into(),
        column,
        priority: Some(2),
        epic: Some("rust-poc".into()),
        ..Issue::default()
    }
}

fn item(id: &str, title: &str, plan: Plan) -> Item {
    Item {
        item: RoadmapItem {
            id: id.into(),
            title: title.into(),
            horizon: "now".into(),
            dod: Some(format!("{title} done")),
            points: 10,
            done: 4,
            first: Some("2026-10-26".into()),
            ..RoadmapItem::default()
        },
        plan,
    }
}

/// Done-when 1: `dash linear sync` writes tasks, roadmap projects, their dates, milestones and relations from Rust, on Linear's
/// recorded answers. An issue Linear holds as the task says is left; one renamed is updated, its state kept of its type; one
/// open again in the running cycle comes out of the archive; a new one is made in its roadmap item's project with the day it was
/// added; a done one made with its past dates goes to the archive; a canceled one never synced is not made. The held roadmap
/// project takes its plan's dates, a new one is made with them, then the plan's milestone and wait are written and read back.
/// An open PR goes through his GitHub integration; linear.edn gets the new issue's key.
#[dash_test]
fn a_sync_on_recorded_answers_writes_tasks_projects_dates_milestones_and_relations() {
    let mut linear = Linear::new();
    let rec = linear.recorded.clone();
    let roadmap_rust = (rec["team"]["teams"]["nodes"][0]["projects"]["nodes"]
        .as_array()
        .unwrap()
        .iter())
    .find(|p| p["name"] == "Roadmap: The Rust tool builds itself")
    .and_then(|p| text(&p["id"]))
    .expect("the held roadmap project");

    let same_one = node(&rec, "AAA-117991");
    let (why, done_when) = said_of(same_one);
    let unchanged = Issue {
        title: text(&same_one["title"]).unwrap(),
        why,
        done_when,
        ..issue("rust-rfc-check-stable", "todo")
    };
    let in_review = Issue {
        title: "A test attribute adds the test budget itself".into(),
        priority: Some(1),
        prs: vec![pr(1506, "open"), pr(1507, "open")],
        ..issue("rust-test-budget-attr", "in-review")
    };
    let open_again = Issue {
        priority: Some(1),
        ..issue("rust-brief-merge-push-order", "todo")
    };
    let new = Issue {
        title: "dash linear sync is Rust's".into(),
        added: Some("2026-11-11".into()),
        project: Some("Roadmap: The Rust tool builds itself".into()),
        labels: vec!["Claude".into()],
        ..issue("rust-linear-sync-port", "todo")
    };
    let done_long_ago = Issue {
        added: Some("2026-11-01".into()),
        done: Some("2026-11-02: merged".into()),
        ..issue("done-new", "done")
    };
    let dropped = issue("old-dropped", "canceled");
    let issues = vec![
        unchanged,
        in_review,
        open_again,
        new,
        done_long_ago,
        dropped,
    ];

    let rust = Plan {
        target: Some("2026-12-15".into()),
        milestones: vec![("2026-11-20".into(), "Design system".into())],
        ..Plan::default()
    };
    let jobs = Plan {
        start: Some("2026-11-10".into()),
        target: Some("2026-11-30".into()),
        after: vec!["rust".into()],
        ..Plan::default()
    };
    let items = vec![
        item("rust", "The Rust tool builds itself", rust),
        item("jobs", "Job search", jobs),
    ];

    let mut saved = BTreeMap::new();
    let mut save = |l: &Links| {
        saved = l.clone();
        Ok(())
    };
    let mut post = |q: &str, v: Value| linear.answer(q, &v);
    let (n, notes) = sync(&issues, &items, &mut post, noon(), &mut save).expect("a sync");
    let said = said(&n);
    let calls = linear.calls.clone();
    let wrote = |f: &str| -> Vec<&Value> {
        calls
            .iter()
            .filter(|(c, _)| c == f)
            .map(|(_, v)| v)
            .collect()
    };

    // tasks
    assert_eq!(
        (n.created, n.updated, n.same, n.archived, n.closed),
        (1, 2, 1, 1, 1),
        "{said}\n{calls:#?}"
    );
    let made = wrote("issueCreate");
    let new_one = made
        .iter()
        .find(|v| v["in"]["title"] == "dash linear sync is Rust's")
        .expect("the new issue made");
    assert_eq!(
        new_one["in"]["projectId"],
        json!(roadmap_rust),
        "in its roadmap item's project"
    );
    assert_eq!(
        new_one["in"]["createdAt"], "2026-11-11T08:00:00Z",
        "made the day it was added"
    );
    let claude = (rec["team"]["teams"]["nodes"][0]["labels"]["nodes"]
        .as_array()
        .unwrap()
        .iter())
    .find(|l| l["name"] == "Claude")
    .map(|l| l["id"].clone())
    .expect("the Claude label");
    assert_eq!(
        new_one["in"]["labelIds"],
        json!([claude]),
        "the agent's label"
    );
    let old = made
        .iter()
        .find(|v| v["in"]["title"] == "done-new")
        .expect("the past done task made");
    assert_eq!(old["in"]["completedAt"], "2026-11-02T18:00:00Z", "{old}");
    assert_eq!(
        wrote("issueArchive").len(),
        1,
        "the past done one archived: {calls:#?}"
    );
    assert_eq!(
        wrote("issueUnarchive").len(),
        1,
        "the one open again taken out: {calls:#?}"
    );
    let renamed = wrote("issueUpdate");
    let review = renamed
        .iter()
        .find(|v| v["in"]["title"] == "A test attribute adds the test budget itself")
        .expect("renamed");
    assert_eq!(
        review["in"]["stateId"], "6283af03-454f-4344-9861-537e112d1098",
        "its In Review state kept: {review}"
    );
    assert!(
        !calls
            .iter()
            .any(|(_, v)| v.to_string().contains("old-dropped")),
        "a canceled task never synced is not made"
    );
    assert_eq!(
        saved["rust-linear-sync-port"].0, "AAA-900000",
        "linear.edn names the new issue: {saved:?}"
    );
    assert_eq!(saved["rust-rfc-check-stable"].0, "AAA-117991");

    // roadmap projects and their dates
    let updated = wrote("projectUpdate");
    assert_eq!(updated.len(), 1, "{calls:#?}");
    assert_eq!(updated[0]["id"], json!(roadmap_rust));
    assert_eq!(
        (
            updated[0]["in"]["startDate"].as_str(),
            updated[0]["in"]["targetDate"].as_str()
        ),
        (Some("2026-10-26"), Some("2026-12-15"))
    );
    assert_eq!(
        updated[0]["in"]["statusId"], "20cd1967-0160-43b9-8e26-9e290ebb5026",
        "a Now item is started"
    );
    let created = wrote("projectCreate");
    let jobs = created
        .iter()
        .find(|v| v["in"]["name"] == "Roadmap: Job search")
        .expect("the new item's project");
    assert_eq!(
        (
            jobs["in"]["startDate"].as_str(),
            jobs["in"]["targetDate"].as_str()
        ),
        (Some("2026-11-10"), Some("2026-11-30"))
    );

    // milestones and relations, read back
    let ms = wrote("projectMilestoneCreate");
    assert_eq!(
        ms[0]["in"],
        json!({"projectId": roadmap_rust, "name": "Design system", "targetDate": "2026-11-20"})
    );
    let rel = wrote("projectRelationCreate");
    assert_eq!(
        rel[0]["in"]["projectId"],
        json!(roadmap_rust),
        "the item it waits on is the source"
    );
    assert!(
        notes
            .iter()
            .any(|l| l == "jobs: Linear holds milestones none; waits on rust"),
        "{notes:#?}"
    );
    assert!(
        notes
            .iter()
            .any(|l| l == "rust: Linear holds milestones 2026-11-20 Design system; waits on none"),
        "{notes:#?}"
    );

    // PRs: the open one not yet held goes through his integration; the one held by another writer is left
    let linked = wrote("attachmentLinkGitHubPR");
    assert_eq!(linked.len(), 1, "{calls:#?}");
    assert_eq!(
        linked[0]["url"],
        "https://github.com/nha-org/site_perso/pull/1507"
    );
    assert_eq!(n.attached, 1);
    println!("{said}\n{}", notes.join("\n"));
}

/// Linear's rewrites of what it stores, read the same on both sides: else 24 issues were sent again at every sync.
#[dash_test]
fn plain_reads_a_text_as_linear_writes_it_back() {
    let sent = "1. brief.rs rule 7: a_b, _quoted_ and [task](tasks.edn) or [site](https://x.y) \n- one\n  - two\n";
    let back = "1. [brief.rs](<http://brief.rs>) rule 7: a\\_b, *quoted* and task or [site](https://x.y)\n* one\n  * two";
    assert_eq!(plain(sent), plain(back));
    assert_eq!(
        plain("snake_case_name stays"),
        "snake_case_name stays",
        "an underscore inside a word is no emphasis"
    );
    assert_eq!(
        mark("Why\n\nsite\\_perso: `a-b`"),
        Some("a-b".into()),
        "the mark read through Linear's escape"
    );
}

/// The board's column, and the one Linear's state follows: an open PR is In Review, a claim In Progress, a merged closing PR
/// with none open Done before the task's :done lands, unless the task recurs.
#[dash_test]
fn a_task_reads_its_column_from_its_stage_claim_and_prs() {
    let task = |stage: &str, prs: Vec<LinkedPr>| LinearTask {
        id: "t".into(),
        stage: stage.into(),
        owner: "claude".into(),
        prs,
        ..LinearTask::default()
    };
    let col = |t: LinearTask, claimed: bool| {
        let held: HashSet<String> = claimed.then(|| t.id.clone()).into_iter().collect();
        issues(vec![t], &held, &[]).remove(0).column.to_string()
    };
    assert_eq!(col(task("open", vec![pr(1, "open")]), false), "in-review");
    assert_eq!(col(task("open", vec![]), true), "started");
    assert_eq!(col(task("dropped", vec![]), false), "canceled");
    assert_eq!(col(task("backlog", vec![]), false), "backlog");
    let draft = LinkedPr {
        draft: true,
        ..pr(2, "open")
    };
    assert_eq!(
        col(task("done", vec![draft]), false),
        "started",
        "done with its closing PR still open, a draft"
    );
    let merged = issues(
        vec![task("open", vec![pr(3, "merged")])],
        &HashSet::new(),
        &[],
    )
    .remove(0);
    assert_eq!((merged.column, state_column(&merged)), ("todo", "done"));
    let claimed = issues(
        vec![task("open", vec![])],
        &HashSet::from(["t".to_string()]),
        &[],
    )
    .remove(0);
    assert_eq!(claimed.column, "started", "a Rust run's claim holds it");
    assert_eq!(
        claimed.labels,
        vec!["Claude".to_string()],
        "the agent's label from its owner"
    );
    let on_now = LinearTask {
        item: Some("rust".into()),
        ..task("open", vec![])
    };
    let on_now = issues(
        vec![on_now],
        &HashSet::new(),
        &[item("rust", "The Rust tool", Plan::default())],
    )
    .remove(0);
    assert_eq!(
        (on_now.tier, on_now.project.as_deref()),
        (Some(1), Some("Roadmap: The Rust tool")),
        "a Now item's tier"
    );
    assert_eq!(
        issues(vec![task("open", vec![])], &HashSet::new(), &[])
            .remove(0)
            .tier,
        None,
        "on no item, last"
    );
    let recurring = Issue {
        every: Some("1 week".into()),
        ..merged
    };
    assert_eq!(state_column(&recurring), "todo");
}

/// Under the free plan's cap, the open tasks ranked last wait and the oldest closed are archived: a run at work before todo,
/// then the pick order's tier, priority and smaller estimate, newest first.
#[dash_test]
fn the_room_keeps_the_open_tasks_ranked_first_under_the_cap() {
    let mut all: Vec<Issue> = (0..ROOM + 2)
        .map(|n| Issue {
            added: Some(format!("2026-10-{:02}", n % 28 + 1)),
            priority: Some(4),
            ..issue(&format!("t{n}"), "todo")
        })
        .collect();
    all.push(Issue {
        priority: Some(4),
        ..issue("running", "started")
    });
    all.push(Issue {
        done: Some("2026-11-01: merged".into()),
        ..issue("closed", "done")
    });
    let (waiting, evict) = room(&all, &Have::new(), 0, noon());
    assert_eq!(waiting.len(), 3, "{waiting:?}");
    assert!(
        !waiting.contains("running"),
        "a run at work keeps its place"
    );
    assert!(evict.contains("closed"), "no room left for a closed one");
    // the pick order's keys (rfc-rust-unified 20g): a Now item's task before tasks on no item, and the smaller estimate first
    let mut picked = all.clone();
    picked.push(Issue {
        tier: Some(1),
        priority: Some(4),
        ..issue("now-item", "todo")
    });
    picked.push(Issue {
        estimate: Some(1),
        ..issue("small", "todo")
    });
    let (waiting, _) = room(&picked, &Have::new(), 0, noon());
    assert!(
        !waiting.contains("now-item") && !waiting.contains("small"),
        "{waiting:?}"
    );
    assert_eq!(waiting.len(), 5, "{waiting:?}");
    let (waiting, _) = room(&all, &Have::new(), 5, noon());
    assert_eq!(
        waiting.len(),
        8,
        "an issue made in Linear alone takes room too"
    );
}

/// The edges /code-review found: the older module's example issues keep their links; an integration call left unanswered is
/// tried again next run, never stood in for by a plain link; a held link gone already is no refusal; a PR Linear refuses is said,
/// never a halt; a cycle started but not yet read back counts from now.
#[dash_test]
fn the_review_s_edges_hold() {
    let node = |key: &str| json!({"id": format!("id-{key}"), "identifier": key, "url": format!("https://linear.app/x/{key}")});
    let have: Have = [
        ("a", node("a")),
        ("example-1", node("example-1")),
        ("gone", node("gone")),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    let kept = links(&have, &[issue("a", "todo")]);
    assert_eq!(
        kept.keys().collect::<Vec<_>>(),
        vec!["a", "example-1"],
        "a deleted task's link goes, an example's stays"
    );

    let open = pr(9, "open");
    let held = json!({"id": "att", "url": open.url, "subtitle": "open", "sourceType": "api"});
    let mut calls = vec![];
    let mut post = |q: &str, _v: Value| -> Result<Value, String> {
        calls.push(
            q.split_once('{')
                .map_or("", |(_, b)| b)
                .trim()
                .split('(')
                .next()
                .unwrap_or_default()
                .to_string(),
        );
        match calls.last().map(String::as_str) {
            Some("attachmentDelete") => Err("linear: Entity not found: Attachment".into()),
            Some("attachmentLinkGitHubPR") => {
                Err("linear: the API failed on « attachmentLinkGitHubPR »: timeout".into())
            }
            _ => Ok(json!({"attachmentCreate": {"success": true}})),
        }
    };
    let said = link(&mut post, "id-a", &open, Some(&held));
    assert!(said.as_ref().is_err_and(|e| unanswered(e)), "{said:?}");
    assert_eq!(
        calls,
        vec!["attachmentDelete", "attachmentLinkGitHubPR"],
        "no plain link stands in for a timeout"
    );

    let i = Issue {
        prs: vec![pr(10, "merged")],
        ..issue("a", "done")
    };
    let mut refuse = |_: &str, _: Value| -> Result<Value, String> {
        Err("linear: Argument Validation Error".into())
    };
    let (n, refused) = attach(&mut refuse, &have, &[i], noon());
    assert_eq!((n, refused.len()), (0, 1), "{refused:?}");

    let mut started = |q: &str, _: Value| -> Result<Value, String> {
        Ok(match q.trim_start().starts_with("mutation") {
            true => json!({"cycleStartUpcomingCycleToday": {"success": true}}),
            false => {
                json!({"team": {"activeCycle": null, "cycles": {"nodes": [{"id": "c2", "startsAt": "2026-11-15T23:00:00.000Z"}]}}})
            }
        })
    };
    let cycle = running_cycle(&mut started, "t", noon()).expect("a cycle");
    assert_eq!(
        cycle,
        Some(json!({"id": "c2", "startsAt": "2026-11-12T12:00:00Z"}))
    );
}

/// `OLDER_TICK` says what the older app starts: true while application.ex starts `Dashboard.Linear.Every`, false once task
/// rust-older-jobs-stop removes it. Out of step, no one writes the issues, or two writers rewrite each other's fields
/// (/code-review). The public mirror holds no application.ex and checks nothing.
#[dash_test]
fn older_tick_says_what_the_older_app_starts() {
    let Some(monorepo) = dash_meter::monorepo() else {
        return;
    };
    let app = monorepo.join("private/dashboard/lib/dashboard/application.ex");
    let app = std::fs::read_to_string(&app).unwrap_or_default();
    assert_eq!(
        app.contains("Dashboard.Linear.Every,"),
        OLDER_TICK,
        "application.ex and linear::OLDER_TICK disagree on who writes Linear's issues"
    );
}

/// The title's « one writer per field » (task rust-linear-sync-port's PM check): while the older app's tick still writes the
/// issues and the roadmap projects (`OLDER_TICK`), a sync writes only what Rust alone holds, the milestones and waits; with the
/// tick stopped, the whole sync. Proved on the recorded answers.
#[dash_test]
fn while_the_older_tick_runs_a_sync_writes_only_rust_s_own_fields() {
    let rust = Plan {
        milestones: vec![("2026-11-20".into(), "Design system".into())],
        ..Plan::default()
    };
    let klar = Plan {
        after: vec!["rust".into()],
        ..Plan::default()
    };
    let items = vec![
        item("rust", "The Rust tool builds itself", rust),
        item("klar", "Klar Plus beta", klar),
    ];
    let issues = vec![Issue {
        title: "renamed".into(),
        ..issue("rust-test-budget-attr", "todo")
    }];
    let mut save = |_: &Links| Ok(());

    let mut linear = Linear::new();
    let mut post = |q: &str, v: Value| linear.answer(q, &v);
    let lines = run(&issues, &items, &mut post, noon(), &mut save, true).expect("a run");
    let fields: Vec<&str> = linear.calls.iter().map(|(f, _)| f.as_str()).collect();
    assert_eq!(
        fields,
        vec!["projectMilestoneCreate", "projectRelationCreate"],
        "{lines:#?}"
    );
    assert!(
        lines[0].contains("Rust wrote the milestones and waits only"),
        "{lines:#?}"
    );

    let mut linear = Linear::new();
    let mut post = |q: &str, v: Value| linear.answer(q, &v);
    run(&issues, &items, &mut post, noon(), &mut save, false).expect("a whole run");
    let fields: Vec<&str> = linear.calls.iter().map(|(f, _)| f.as_str()).collect();
    assert!(
        fields.contains(&"issueUpdate") && fields.contains(&"projectUpdate"),
        "{fields:?}"
    );
}

/// A grandparent ranks with its best grandchild, each parent just before its sub-issues (/code-review of PR 1511: one level
/// only was carried, so a backlog grandparent fell past the cut while its started grandchild was kept).
#[dash_test]
fn a_grandparent_ranks_with_its_best_grandchild_just_before_it() {
    let a = issue("a", "backlog");
    let b = Issue {
        parent: Some("a".into()),
        ..issue("b", "backlog")
    };
    let c = Issue {
        parent: Some("b".into()),
        ..issue("c", "started")
    };
    let other = issue("other", "todo");
    let open = [&c, &other, &b, &a];
    let keys: Vec<&str> = rank(&open, &Have::new())
        .iter()
        .map(|i| i.key.as_str())
        .collect();
    assert_eq!(keys, ["a", "b", "c", "other"]);
}
