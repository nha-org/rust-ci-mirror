//! A superapp PR's check on CircleCI while no Hostman box is up (task superapp-ci-circleci; @nha, 2026-11-18: « about CI:
//! add monitoring ? datadog ? if not lmk we find a free CI provider. », then « yes » to CircleCI). superapp.yml's job asks
//! for `[ci]`, so with no box it waits queued. `dash ci superapp <pr>`, run at merge time, is the one way in: nothing runs
//! it on a clock, and no GitHub run is cancelled.
//! - A head green under `CONTEXT` against main's present head is not run again; against an older main it is.
//! - A head whose pipeline still runs is never started twice.
//! - It refuses under `RESERVE` credits left this month on the owner's every project, the pipelines in flight counted at
//!   their worst (`WAIT_S` at the large rate), saying what the last run cost.
//! - It starts `.circleci/config.yml`'s `superapp` workflow on main, main's own config always (`superapp: true`, the PR's
//!   head and number, never the dashboard's `routed`): the job fetches the head and merges the pipeline's main into it.
//! - It waits for that pipeline's `superapp-linux` job, `WAIT_S` at most, a lost read tried again, and posts `CONTEXT` on
//!   that head alone: success only when the job ran and passed, failure on any error or no job, main's sha named. A head
//!   that moved meanwhile gets nothing. On a timeout, or once it gives up, the pipeline is cancelled.
//! - It runs bin/explainer-gate on the head itself (its runner is down too) and says its verdict.
//!
//! dash-forge's checks read `CONTEXT` as the verdict of a superapp job still queued (`checks::handed`).
use dash_core::{Host, Program};
use serde_json::{Value as Json, json};
use std::time::Duration;

/// The status this command posts on a PR's head; CircleCI's own statuses name the job, `ci/circleci: superapp-linux`.
pub const CONTEXT: &str = "ci/circleci: superapp";
/// The status of the explainer's gate run here, read by dash-forge's checks in its queued job's place.
pub const GATE: &str = "dash/explainer-gate";
/// The workflow and its one job in .circleci/config.yml.
const WORKFLOW: &str = "superapp";
const JOB: &str = "superapp-linux";
const OWNER: &str = "nha-org";
const REPO: &str = "nha-org/site_perso";
const SLUG: &str = "gh/nha-org/site_perso";
/// The free plan's month (circleci.com/pricing), the owner's every project counted, and what stays for the month's other
/// runs: under it, no start.
const FREE: i64 = 30_000;
pub const RESERVE: i64 = 3_000;
/// A Linux VM large's credits a minute, the job's resource class.
const LARGE: i64 = 20;
/// How long the verdict is waited for: the free plan ends a job at an hour.
const WAIT_S: i64 = 65 * 60;
const EVERY_S: u64 = 30;
/// How long a created pipeline may list no workflow: CircleCI makes its workflows within seconds of reading the config.
const NO_FLOW_S: i64 = 5 * 60;
/// A read that fails is tried this many times, `EVERY_S` apart, before the command gives up.
const TRIES: usize = 5;
/// A CircleCI workflow still running (API v2's states).
const LIVE: [&str; 3] = ["running", "on_hold", "failing"];

/// gh with these arguments under the host's login: its output.
pub type Gh<'a> = &'a dyn Fn(&[&str]) -> Result<String, String>;
/// CircleCI's API v2 at this path, a POST of the body when one is given: the answer's body (`api`).
pub type Circle<'a> = &'a dyn Fn(&str, Option<&str>) -> Result<String, String>;
/// bin/explainer-gate on a head, given the PR's description and opening time: whether it passed, and what it said.
pub type Gate<'a> = &'a dyn Fn(&str, &str, &str) -> Result<(bool, String), String>;

/// What the command reaches: GitHub, CircleCI, the explainer's gate, a wait, the clock (seconds) and where its lines go.
pub struct Ports<'a> {
    pub gh: Gh<'a>,
    pub circle: Circle<'a>,
    pub gate: Gate<'a>,
    pub sleep: &'a dyn Fn(u64),
    pub now_s: &'a dyn Fn() -> i64,
    pub say: &'a dyn Fn(&str),
}

fn parsed(text: &str) -> Result<Json, String> {
    serde_json::from_str(text).map_err(|e| {
        format!(
            "not JSON ({e}): {}",
            text.chars().take(120).collect::<String>()
        )
    })
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(8)]
}

fn url(n: u64) -> String {
    format!("https://app.circleci.com/pipelines/{SLUG}/{n}")
}

/// A path segment or query value, every byte but the unreserved ones percent-encoded: a workflow's name (another
/// project's may hold a space) and Insights' page token (`+`, `/`, `=`).
fn encoded(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn items(v: &Json) -> impl Iterator<Item = &Json> {
    v["items"].as_array().into_iter().flatten()
}

/// `read` until it answers, `TRIES` times `EVERY_S` apart: one lost answer in an hour's wait never loses the run.
fn retried<T>(p: &Ports, read: impl Fn() -> Result<T, String>) -> Result<T, String> {
    let mut last = String::new();
    for i in 0..TRIES {
        match read() {
            Ok(v) => return Ok(v),
            Err(e) => last = e,
        }
        if i + 1 < TRIES {
            (p.sleep)(EVERY_S);
        }
    }
    Err(format!("{TRIES} tries: {last}"))
}

/// `dash ci superapp <pr>`: PR `pr`'s head checked on CircleCI from main's config, its status posted, then the
/// explainer's gate. Its exit code: 0 both green, 1 either red, 3 no status posted.
pub fn check(p: &Ports, pr: u64) -> Result<i32, String> {
    let read_pr = || parsed(&(p.gh)(&["api", &format!("repos/{REPO}/pulls/{pr}")])?);
    let v = read_pr()?;
    let sha = v["head"]["sha"].as_str().unwrap_or_default().to_string();
    if v["state"] != "open" || sha.is_empty() {
        return Err(format!("PR {pr} is not open"));
    }
    let main = parsed(&(p.gh)(&["api", &format!("repos/{REPO}/commits/main")])?)?;
    let main = main["sha"]
        .as_str()
        .ok_or("main's head unread")?
        .to_string();
    let circleci = circleci(p, pr, &sha, &main, &read_pr)?;
    // the explainer's gate runs on the same runner as the superapp's job: here, never left to a human, its verdict posted
    // as `GATE` on the head, which dash-forge's checks read in its queued job's place; one that could not run is red
    let (body, created) = (
        v["body"].as_str().unwrap_or_default(),
        v["created_at"].as_str().unwrap_or_default(),
    );
    let (passed, said) =
        (p.gate)(&sha, body, created).unwrap_or_else(|e| (false, format!("not run: {e}")));
    let state = if passed { "success" } else { "failure" };
    post(
        p,
        &sha,
        GATE,
        state,
        &format!("bin/explainer-gate on this Mac: {said}"),
        "",
    )?;
    (p.say)(&format!("{GATE} {state} on {}: {said}", short(&sha)));
    Ok(match (circleci, passed) {
        (Some(true), true) => 0,
        (None, _) => 3,
        _ => 1,
    })
}

/// A status on `sha`, tried again when lost; its description cut to GitHub's 140 characters.
fn post(
    p: &Ports,
    sha: &str,
    context: &str,
    state: &str,
    description: &str,
    target: &str,
) -> Result<(), String> {
    let description: String = description.chars().take(140).collect();
    let send = || {
        (p.gh)(&[
            "api",
            "-X",
            "POST",
            &format!("repos/{REPO}/statuses/{sha}"),
            "-f",
            &format!("state={state}"),
            "-f",
            &format!("context={context}"),
            "-f",
            &format!("description={description}"),
            "-f",
            &format!("target_url={target}"),
        ])
    };
    retried(p, send).map(drop)
}

/// The CircleCI half: Some(green) once `CONTEXT` holds a verdict on `sha` against `main`, None when none was posted.
fn circleci(
    p: &Ports,
    pr: u64,
    sha: &str,
    main: &str,
    read_pr: &dyn Fn() -> Result<Json, String>,
) -> Result<Option<bool>, String> {
    // GitHub lists a context's newest status first: a green against main's present head stands
    let statuses = parsed(&(p.gh)(&[
        "api",
        &format!("repos/{REPO}/commits/{sha}/statuses?per_page=100"),
    ])?)?;
    let ours = (statuses.as_array().into_iter().flatten()).find(|s| s["context"] == CONTEXT);
    let against = format!("merged with main {}", short(main));
    if let Some(s) = ours.filter(|s| s["state"] == "success") {
        let description = s["description"].as_str().unwrap_or_default();
        if description.contains(&against) {
            (p.say)(&format!(
                "{CONTEXT} is green on {} {against} already: {}",
                short(sha),
                s["target_url"].as_str().unwrap_or("")
            ));
            return Ok(Some(true));
        }
        (p.say)(&format!(
            "{CONTEXT} is green on {} against an older main ({description}): run again",
            short(sha)
        ));
    }
    let live = in_flight(p)?;
    let (n, id) = match live.iter().find(|f| f.compiled.contains(sha)) {
        // one already runs for this head (a run cut off, another session): its verdict is waited for, never a second run
        Some(f) => {
            (p.say)(&format!(
                "pipeline {} already runs for {}: {}; waited for",
                f.number,
                short(sha),
                url(f.number)
            ));
            (f.number, f.id.clone())
        }
        None => {
            let (left, last) = credits(p)?;
            // the pipelines in flight and the one to start, each at its worst
            let worst = (live.len() as i64 + 1) * WAIT_S / 60 * LARGE;
            (p.say)(&format!(
                "CircleCI: {left} credits left this month, {} min of a large VM; {} in flight and this run count {worst} at worst; {last}",
                left / LARGE,
                live.len()
            ));
            if left - worst < RESERVE {
                (p.say)(&format!(
                    "not started: it would pass under the {RESERVE} credits kept in reserve"
                ));
                return Ok(None);
            }
            let body =
                json!({"branch": "main", "parameters": {"superapp": true, "head": sha, "pr": pr}});
            let got = parsed(&(p.circle)(
                &format!("project/{SLUG}/pipeline"),
                Some(&body.to_string()),
            )?)?;
            let n = (got["number"].as_u64())
                .ok_or(format!("CircleCI gave no pipeline number: {got}"))?;
            (p.say)(&format!("pipeline {n} for {}: {}", short(sha), url(n)));
            (n, got["id"].as_str().unwrap_or_default().to_string())
        }
    };
    let verdict = match verdict(p, &id) {
        Ok(v) => v,
        Err(e) => {
            cancel(p, &id);
            return Err(format!("pipeline {n} cancelled: {e}"));
        }
    };
    // the main it merged: the pipeline's own commit
    let pipeline = retried(p, || parsed(&(p.circle)(&format!("pipeline/{id}"), None)?))?;
    let merged = pipeline["vcs"]["revision"]
        .as_str()
        .unwrap_or(main)
        .to_string();
    // the head it was for, or nothing: a newer head's own check is still to come
    let now = retried(p, read_pr)?;
    if now["head"]["sha"] != sha {
        (p.say)(&format!("PR {pr}'s head moved meanwhile: no status posted"));
        return Ok(None);
    }
    let (state, why) = match &verdict {
        Ok(()) => (
            "success",
            format!(
                "{JOB} passed: {} merged with main {}",
                short(sha),
                short(&merged)
            ),
        ),
        Err(why) => (
            "failure",
            format!("{why}: {} merged with main {}", short(sha), short(&merged)),
        ),
    };
    post(
        p,
        sha,
        CONTEXT,
        state,
        &format!("pipeline {n}: {why}"),
        &url(n),
    )?;
    (p.say)(&format!("{CONTEXT} {state} on {}: {why}", short(sha)));
    Ok(Some(verdict.is_ok()))
}

/// A superapp pipeline in flight on main: its number, its id, and its compiled config, which names the head it tests.
struct Flight {
    number: u64,
    id: String,
    compiled: String,
}

/// The superapp pipelines in flight on main: the API's, started within `WAIT_S`, whose superapp workflow runs or is not
/// made yet. CircleCI's first page of main's pipelines; a pipeline older than the wait is never read again.
fn in_flight(p: &Ports) -> Result<Vec<Flight>, String> {
    let get = |path: String| parsed(&(p.circle)(&path, None)?);
    let since = dash_core::iso((p.now_s)() - WAIT_S);
    let listed = get(format!("project/{SLUG}/pipeline?branch=main"))?;
    let mut live = vec![];
    let recent = |pl: &&Json| {
        pl["trigger"]["type"] == "api"
            && pl["state"] != "errored"
            && pl["created_at"]
                .as_str()
                .is_some_and(|c| c >= since.as_str())
    };
    for pl in items(&listed).filter(recent) {
        let id = pl["id"].as_str().unwrap_or_default();
        let flows = get(format!("pipeline/{id}/workflow"))?;
        let flows: Vec<&Json> = items(&flows).collect();
        let running = flows
            .iter()
            .any(|w| w["name"] == WORKFLOW && LIVE.iter().any(|s| w["status"] == *s));
        if running || flows.is_empty() {
            let config = get(format!("pipeline/{id}/config"))?;
            live.push(Flight {
                number: pl["number"].as_u64().unwrap_or_default(),
                id: id.to_string(),
                compiled: config["compiled"].as_str().unwrap_or_default().to_string(),
            });
        }
    }
    Ok(live)
}

/// Every workflow of pipeline `id` still running, cancelled: a run given up spends no more credits. Best effort.
fn cancel(p: &Ports, id: &str) {
    if let Ok(flows) = (p.circle)(&format!("pipeline/{id}/workflow"), None).and_then(|t| parsed(&t))
    {
        for w in items(&flows).filter(|w| LIVE.iter().any(|s| w["status"] == *s)) {
            let _ = (p.circle)(
                &format!("workflow/{}/cancel", w["id"].as_str().unwrap_or_default()),
                Some("{}"),
            );
        }
    }
}

/// The pipeline's `superapp` workflow's `superapp-linux` job: Ok when it ran and passed, else why not. Only the workflow of
/// that name in the pipeline this command started is read: the dashboard's `check`, or any other, is never taken. Past
/// `WAIT_S` the pipeline is cancelled and it is a failure.
fn verdict(p: &Ports, id: &str) -> Result<Result<(), String>, String> {
    let start = (p.now_s)();
    let until = start + WAIT_S;
    let get = |path: String| retried(p, || parsed(&(p.circle)(&path, None)?));
    loop {
        let pipeline = get(format!("pipeline/{id}"))?;
        if pipeline["state"] == "errored" {
            let why = pipeline["errors"][0]["message"]
                .as_str()
                .unwrap_or("no reason given");
            return Ok(Err(format!(
                "the pipeline errored: {}",
                why.chars().take(80).collect::<String>()
            )));
        }
        let flows = get(format!("pipeline/{id}/workflow"))?;
        let flows: Vec<&Json> = items(&flows).collect();
        let ours = flows.iter().find(|w| w["name"] == WORKFLOW);
        match (ours, flows.is_empty()) {
            (None, false) => return Ok(Err(format!("no {WORKFLOW} workflow in the pipeline"))),
            (Some(w), _) => match w["status"].as_str().unwrap_or_default() {
                "success" => {
                    let jobs = get(format!(
                        "workflow/{}/job",
                        w["id"].as_str().unwrap_or_default()
                    ))?;
                    let ran = items(&jobs).any(|j| j["name"] == JOB && j["status"] == "success");
                    return Ok(if ran {
                        Ok(())
                    } else {
                        Err(format!("no {JOB} job ran"))
                    });
                }
                "running" | "on_hold" | "failing" | "" => {}
                other => return Ok(Err(format!("the {WORKFLOW} workflow {other}"))),
            },
            // a created pipeline with no workflow past a few minutes: main's config holds no superapp workflow
            (None, true) if pipeline["state"] == "created" && (p.now_s)() >= start + NO_FLOW_S => {
                return Ok(Err(format!(
                    "no workflow in the pipeline after {} min: main's .circleci/config.yml holds no {WORKFLOW} workflow",
                    NO_FLOW_S / 60
                )));
            }
            (None, true) => {}
        }
        if (p.now_s)() >= until {
            cancel(p, id);
            return Ok(Err(format!("no verdict in {} min, cancelled", WAIT_S / 60)));
        }
        (p.sleep)(EVERY_S);
    }
}

/// The credits used this month on the owner's every project (the free plan's credits are the organisation's): each
/// workflow's runs since the 1st, `credits_used` at its real rate (a large VM's 20 a minute), read run by run from
/// Insights (`workflows/{name}` takes start-date and end-date; the workflows' summary only a reporting window, so it
/// names the workflows alone, the superapp's always counted, as Insights may list a new one late). And what the last
/// superapp run cost.
fn credits(p: &Ports) -> Result<(i64, String), String> {
    let now = dash_core::iso((p.now_s)());
    let window = format!(
        "all-branches=true&start-date={}-01T00:00:00Z&end-date={now}",
        &now[..7]
    );
    let get = |path: String| parsed(&(p.circle)(&path, None)?);
    let projects = get(format!(
        "insights/gh/{OWNER}/summary?reporting-window=last-90-days"
    ))?;
    let (mut used, mut last) = (0, "no superapp run yet this month".to_string());
    for project in
        (projects["all_projects"].as_array().into_iter().flatten()).filter_map(Json::as_str)
    {
        let slug = format!("gh/{OWNER}/{project}");
        let summary = get(format!(
            "insights/{slug}/workflows?all-branches=true&reporting-window=last-90-days"
        ))?;
        let mut names: Vec<&str> = items(&summary).filter_map(|w| w["name"].as_str()).collect();
        if slug == SLUG && !names.contains(&WORKFLOW) {
            names.push(WORKFLOW);
        }
        for name in names {
            let path = format!("insights/{slug}/workflows/{}?{window}", encoded(name));
            let (credits, newest) = runs(p, &path)?;
            used += credits;
            if let (true, Some(r)) = (slug == SLUG && name == WORKFLOW, newest) {
                last = r;
            }
        }
    }
    Ok((FREE - used, last))
}

/// One workflow's credits over its pages of runs, and its newest run's cost in words.
fn runs(p: &Ports, path: &str) -> Result<(i64, Option<String>), String> {
    let (mut used, mut newest, mut token) = (0, None, String::new());
    // ponytail: 20 pages of runs a workflow, a month's at today's rate many times over
    for _ in 0..20 {
        let page = if token.is_empty() {
            String::new()
        } else {
            format!("&page-token={}", encoded(&token))
        };
        let runs = parsed(&(p.circle)(&format!("{path}{page}"), None)?)?;
        if let (true, Some(r)) = (newest.is_none(), items(&runs).next()) {
            let (credits, min) = (
                r["credits_used"].as_i64().unwrap_or_default(),
                r["duration"].as_i64().unwrap_or_default() / 60,
            );
            newest = Some(format!(
                "the last superapp run: {credits} credits, {min} min"
            ));
        }
        used += items(&runs)
            .filter_map(|r| r["credits_used"].as_i64())
            .sum::<i64>();
        match runs["next_page_token"].as_str() {
            Some(t) if !t.is_empty() => token = t.to_string(),
            _ => break,
        }
    }
    Ok((used, newest))
}

/// CircleCI's API through the host's programs: the token bin/secret holds under `circleci`, read at the first call, goes in
/// curl's environment alone, never its command line or a log.
pub fn api(host: &dyn Host) -> impl Fn(&str, Option<&str>) -> Result<String, String> + '_ {
    let token = std::cell::OnceCell::new();
    move |path, body| {
        // his circleci.env names it CIRCLE_API_TOKEN, bin/secret's renewal CIRCLE_TOKEN: either reads, as the older app's
        let key = token.get_or_init(|| {
            secret(host, "CIRCLE_API_TOKEN").or_else(|_| secret(host, "CIRCLE_TOKEN"))
        });
        curl(host, key.as_deref().map_err(Clone::clone)?, path, body)
    }
}

fn program<'a>(
    bin: &'a str,
    args: &'a [&'a str],
    dir: Option<&'a std::path::Path>,
    given: &'a [(&'a str, &'a str)],
    s: u64,
) -> Program<'a> {
    Program {
        bin,
        args,
        dir,
        given,
        stdin: None,
        limit: Duration::from_secs(s),
    }
}

/// bin/secret's `circleci` value under `var`.
fn secret(host: &dyn Host, var: &str) -> Result<String, String> {
    let bin = host
        .repo()
        .join("bin/secret")
        .to_string_lossy()
        .into_owned();
    let ran = host.run(&program(&bin, &["get", "circleci", var], None, &[], 15))?;
    let value = ran.stdout.trim().to_string();
    match ran.code == 0 && !value.is_empty() {
        true => Ok(value),
        false => Err(format!("bin/secret get circleci {var}: none")),
    }
}

fn curl(host: &dyn Host, key: &str, path: &str, body: Option<&str>) -> Result<String, String> {
    let url = format!("https://circleci.com/api/v2/{path}");
    let mut args = vec![
        "-sS",
        "--max-time",
        "30",
        "--proto",
        "=https",
        "--variable",
        "%KEY",
    ];
    args.extend([
        "--expand-header",
        "Circle-Token: {{KEY}}",
        "-w",
        "\n%{http_code}",
    ]);
    if let Some(body) = body {
        args.extend(["-H", "content-type: application/json", "--data", body]);
    }
    args.push(&url);
    let ran = host.run(&program("curl", &args, None, &[("KEY", key)], 40))?;
    let (said, code) = ran
        .stdout
        .trim_end()
        .rsplit_once('\n')
        .unwrap_or(("", ran.stdout.trim()));
    match code.parse::<u16>() {
        Ok(200..=299) => Ok(said.to_string()),
        Ok(code) => Err(format!(
            "CircleCI {path}: HTTP {code}: {}",
            said.chars().take(160).collect::<String>()
        )),
        Err(_) => Err(format!("CircleCI {path}: no answer: {}", ran.stderr.trim())),
    }
}

/// bin/explainer-gate on `head` against origin/main in the host's checkout, both fetched first, as explainer-gate.yml
/// runs it with the PR's description and opening time.
fn gate(host: &dyn Host, head: &str, body: &str, created: &str) -> Result<(bool, String), String> {
    let repo = host.repo();
    // origin is GitHub over ssh: his agent's socket, the one thing of this environment the fetch needs
    let agent = std::env::var("SSH_AUTH_SOCK").unwrap_or_default();
    let ssh = [("SSH_AUTH_SOCK", agent.as_str())];
    let ssh: &[(&str, &str)] = if agent.is_empty() { &[] } else { &ssh };
    let fetched = host.run(&program(
        "git",
        &["fetch", "-q", "origin", "main", head],
        Some(&repo),
        ssh,
        120,
    ))?;
    if fetched.code != 0 {
        return Err(format!(
            "git fetch of main and {}: {}",
            short(head),
            fetched.stderr.trim()
        ));
    }
    let given = [("PR_BODY", body), ("PR_CREATED", created)];
    let bin = repo
        .join("bin/explainer-gate")
        .to_string_lossy()
        .into_owned();
    let ran = host.run(&program(
        &bin,
        &["origin/main", head],
        Some(&repo),
        &given,
        120,
    ))?;
    let said = format!("{} {}", ran.stdout.trim(), ran.stderr.trim());
    let said: String = said
        .trim()
        .lines()
        .last()
        .unwrap_or_default()
        .chars()
        .take(200)
        .collect();
    Ok((ran.code == 0, said))
}

const USAGE: &str = "dash ci superapp <pr> (while no Hostman box is up, at merge time: the PR's head merged with main on CircleCI, its verdict posted as ci/circleci: superapp, then bin/explainer-gate on it; waits up to 65 min; exit 0 green, 1 red, 3 no status)";

fn run(host: &dyn Host, args: &[&str], out: &mut dyn std::io::Write) {
    let Some(pr) = (match args {
        [pr] => pr.parse().ok(),
        _ => None,
    }) else {
        eprintln!("usage: {USAGE}");
        std::process::exit(2)
    };
    let circle = api(host);
    let gh = |args: &[&str]| host.forge(args);
    let gate = |head: &str, body: &str, created: &str| gate(host, head, body, created);
    let now_s = || host.now_ms() / 1000;
    let sleep = |s: u64| std::thread::sleep(Duration::from_secs(s));
    let out = std::cell::RefCell::new(out);
    let say = |l: &str| {
        let mut out = out.borrow_mut();
        let _ = writeln!(out, "{l}");
        let _ = out.flush();
    };
    let ports = Ports {
        gh: &gh,
        circle: &circle,
        gate: &gate,
        sleep: &sleep,
        now_s: &now_s,
        say: &say,
    };
    let code = check(&ports, pr).unwrap_or_else(|e| {
        say(&format!("no status posted: {e}"));
        3
    });
    std::process::exit(code)
}

dash_core::submit! { dash_core::Command { usage: USAGE, words: &["ci", "superapp"], run: Some(run) } }

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;
    use std::cell::{Cell, RefCell};

    const HEAD: &str = "aaaa1111bbbb";
    /// Insights' answer for a workflow's runs, as CircleCI gave it on 2026-11-19 (two items kept).
    const RECORDED: &str = r#"{"next_page_token": null, "items": [{"id": "99398b82-f656-4648-8193-e3d427fcac4c", "duration": 227, "status": "failed", "created_at": "2026-11-19T00:48:21.264Z", "stopped_at": "2026-11-19T00:52:08.455Z", "credits_used": 75, "branch": "task/superapp-ci-circleci", "is_approval": false}, {"id": "b1878ae2-62ec-43b6-9d5c-c927feb7fabd", "duration": 46, "status": "canceled", "created_at": "2026-11-19T00:44:33.869Z", "stopped_at": "2026-11-19T00:45:20.043Z", "credits_used": 14, "branch": "task/superapp-ci-circleci", "is_approval": false}]}"#;

    /// A stand-in world: PR 5 open at `heads` in turn, CircleCI's new pipeline answering `flows` then `jobs`, `running`
    /// superapp pipelines on main with the heads they test, `statuses` already on the head, `lost` CircleCI reads failing
    /// first, the explainer's gate `gate`. What was asked is kept, and what was said.
    struct World {
        heads: RefCell<Vec<&'static str>>,
        flows: Json,
        jobs: Json,
        pipeline: Json,
        superapp_runs: String,
        statuses: Json,
        running: Vec<&'static str>,
        r_reads: Cell<usize>,
        lost: Cell<usize>,
        gate: bool,
        asked: RefCell<Vec<String>>,
        said: RefCell<Vec<String>>,
    }

    impl World {
        fn new(flows: Json, jobs: Json) -> Self {
            World {
                heads: RefCell::new(vec![HEAD, HEAD]),
                flows,
                jobs,
                pipeline: json!({"state": "created", "vcs": {"revision": "mmmm2222ffff"}}),
                superapp_runs: RECORDED.into(),
                statuses: json!([]),
                running: vec![],
                r_reads: Cell::new(0),
                lost: Cell::new(0),
                gate: true,
                asked: RefCell::default(),
                said: RefCell::default(),
            }
        }

        fn run(&self) -> i32 {
            let gh = |args: &[&str]| -> Result<String, String> {
                self.asked
                    .borrow_mut()
                    .push(format!("gh {}", args.join(" ")));
                Ok(match args.last().copied().unwrap_or_default() {
                    p if p.ends_with("/pulls/5") => {
                        let mut heads = self.heads.borrow_mut();
                        let h = if heads.len() > 1 {
                            heads.remove(0)
                        } else {
                            heads[0]
                        };
                        json!({"state": "open", "head": {"sha": h}, "body": "b", "created_at": "c"})
                            .to_string()
                    }
                    p if p.ends_with("/commits/main") => json!({"sha": "mmmm2222ffff"}).to_string(),
                    p if p.contains("/statuses?") => self.statuses.to_string(),
                    _ => "{}".into(),
                })
            };
            let circle = |path: &str, body: Option<&str>| -> Result<String, String> {
                self.asked
                    .borrow_mut()
                    .push(format!("circle {path} {}", body.unwrap_or("")));
                if path.starts_with("pipeline/p1") && self.lost.get() > 0 {
                    self.lost.set(self.lost.get() - 1);
                    return Err("CircleCI: HTTP 502".into());
                }
                // in flight: started now; one an hour and more old is never read
                let now = dash_core::iso(1_795_000_000);
                let mut live: Vec<Json> = (self.running.iter().enumerate())
                    .map(|(i, _)| json!({"id": format!("r{i}"), "number": 30 + i, "trigger": {"type": "api"}, "state": "created", "created_at": now}))
                    .collect();
                live.push(json!({"id": "old", "number": 9, "trigger": {"type": "api"}, "state": "created", "created_at": "2026-01-01T00:00:00Z"}));
                Ok(match path {
                    p if p.starts_with("insights/gh/nha-org/summary") => {
                        json!({"all_projects": ["site_perso", "klar"]}).to_string()
                    }
                    // Insights may list the superapp's workflow late: it is counted all the same
                    p if p.contains("/workflows?") => {
                        json!({"items": [{"name": "dashboard"}]}).to_string()
                    }
                    p if p.starts_with("insights/gh/nha-org/site_perso/workflows/superapp?") => {
                        self.superapp_runs.clone()
                    }
                    // another project's spend counts against the same month
                    p if p.starts_with("insights/gh/nha-org/klar/workflows/dashboard?") => {
                        json!({"items": [{"credits_used": 500}]}).to_string()
                    }
                    p if p.starts_with("insights/") => {
                        json!({"items": [{"credits_used": 0}]}).to_string()
                    }
                    "project/gh/nha-org/site_perso/pipeline?branch=main" => {
                        json!({"items": live}).to_string()
                    }
                    p if p.starts_with("pipeline/old") => panic!("an old pipeline read: {p}"),
                    // running when listed, then the first one's verdict for whoever waits on it
                    p if p.starts_with("pipeline/r") && p.ends_with("/workflow") => {
                        let first = p == "pipeline/r0/workflow";
                        let status = if !first || self.r_reads.replace(self.r_reads.get() + 1) == 0
                        {
                            "running"
                        } else {
                            "success"
                        };
                        json!({"items": [{"name": "superapp", "id": "wr", "status": status}]})
                            .to_string()
                    }
                    p if p.starts_with("pipeline/r") && !p.contains("/config") => {
                        self.pipeline.to_string()
                    }
                    p if p.starts_with("pipeline/r") && p.ends_with("/config") => {
                        let i: usize = p["pipeline/r".len()..]
                            .split('/')
                            .next()
                            .unwrap_or("0")
                            .parse()
                            .unwrap_or(0);
                        json!({"compiled": format!("git fetch origin {}", self.running[i])})
                            .to_string()
                    }
                    "project/gh/nha-org/site_perso/pipeline" => {
                        json!({"number": 42, "id": "p1"}).to_string()
                    }
                    "pipeline/p1" => self.pipeline.to_string(),
                    "pipeline/p1/workflow" => self.flows.to_string(),
                    p if p.ends_with("/job") => self.jobs.to_string(),
                    _ => "{}".into(),
                })
            };
            let gate = |head: &str, _: &str, _: &str| Ok((self.gate, format!("gate on {head}")));
            let say = |l: &str| self.said.borrow_mut().push(l.to_string());
            // the clock moves as the command waits
            let clock = Cell::new(1_795_000_000i64);
            let sleep = |s: u64| clock.set(clock.get() + s as i64);
            let now_s = || clock.get();
            let ports = Ports {
                gh: &gh,
                circle: &circle,
                gate: &gate,
                sleep: &sleep,
                now_s: &now_s,
                say: &say,
            };
            check(&ports, 5).unwrap_or_else(|e| {
                self.said
                    .borrow_mut()
                    .push(format!("no status posted: {e}"));
                3
            })
        }

        /// The statuses posted under `context`.
        fn posted_as(&self, context: &str) -> Vec<String> {
            (self.asked.borrow().iter())
                .filter(|l| {
                    l.contains("POST repos/nha-org/site_perso/statuses/")
                        && l.contains(&format!("context={context} "))
                })
                .cloned()
                .collect()
        }

        fn posted(&self) -> Vec<String> {
            self.posted_as(CONTEXT)
        }

        fn started(&self) -> bool {
            self.asked
                .borrow()
                .iter()
                .any(|l| l.contains("site_perso/pipeline {"))
        }

        fn cancelled(&self) -> bool {
            self.asked.borrow().iter().any(|l| l.contains("/cancel"))
        }
    }

    fn superapp(status: &str) -> Json {
        json!({"items": [{"name": "superapp", "id": "w1", "status": status}]})
    }

    fn job(status: &str) -> Json {
        json!({"items": [{"name": "superapp-linux", "status": status}]})
    }

    /// Done-when of task superapp-ci-circleci: main's config always (branch main), the PR's head and number, `superapp`
    /// alone (never the dashboard's `routed`); a passed job is a success on that head naming the main it merged; the
    /// credits are the owner's every project's, the recorded Insights answer read; a lost read is tried again; the
    /// explainer's gate runs here and says its verdict.
    #[dash_test]
    fn a_passed_superapp_job_is_a_success_on_the_head_it_ran_for() {
        let w = World::new(superapp("success"), job("success"));
        w.lost.set(2);
        assert_eq!(w.run(), 0, "{:#?}", w.said.borrow());
        let trigger =
            r#"{"branch":"main","parameters":{"head":"aaaa1111bbbb","pr":5,"superapp":true}}"#;
        assert!(
            w.asked.borrow().iter().any(|l| l.ends_with(trigger)),
            "{:#?}",
            w.asked.borrow()
        );
        assert_eq!(w.posted().len(), 1);
        assert!(w.posted()[0].contains(&format!(
            "statuses/{HEAD} -f state=success -f context=ci/circleci: superapp"
        )));
        assert!(
            w.posted()[0].contains("merged with main mmmm2222"),
            "{:?}",
            w.posted()
        );
        let said = w.said.borrow();
        // 30000 - 75 - 14 (recorded) - 500 (klar); this run at its worst, 65 min x 20
        assert_eq!(
            said[0],
            "CircleCI: 29411 credits left this month, 1470 min of a large VM; 0 in flight and this run count 1300 at worst; the last superapp run: 75 credits, 3 min",
        );
        assert!(
            said[1].starts_with("pipeline 42 for aaaa1111"),
            "said at once: {said:#?}"
        );
        assert_eq!(
            said.last().map(String::as_str),
            Some("dash/explainer-gate success on aaaa1111: gate on aaaa1111bbbb")
        );
        let gate = w.posted_as(GATE);
        assert!(
            gate.len() == 1 && gate[0].contains("state=success"),
            "{gate:?}"
        );
        assert!(!w.cancelled());
    }

    /// The matcher: a pipeline holding the dashboard's workflow alone is never taken as the superapp's, and a superapp
    /// workflow whose job never ran is no pass: both post a failure.
    #[dash_test]
    fn a_dashboard_pipeline_or_a_job_that_never_ran_is_never_taken() {
        let dashboard = json!({"items": [{"name": "dashboard", "id": "w9", "status": "success"}]});
        let w = World::new(dashboard, job("success"));
        assert_eq!(w.run(), 1);
        assert!(w.posted()[0].contains("state=failure"), "{:?}", w.posted());
        let w = World::new(
            superapp("success"),
            json!({"items": [{"name": "check", "status": "success"}]}),
        );
        assert_eq!(w.run(), 1);
        assert!(
            w.posted()[0].contains("state=failure")
                && w.posted()[0].contains("no superapp-linux job ran")
        );
    }

    /// The error path: an errored pipeline or a failed workflow posts a failure; reads lost past their tries cancel the
    /// pipeline and post nothing; a run past the hour is cancelled and a failure.
    #[dash_test]
    fn an_error_posts_a_failure_and_a_run_given_up_is_cancelled() {
        let mut w = World::new(superapp("success"), job("success"));
        w.pipeline = json!({"state": "errored", "errors": [{"message": "Unexpected argument(s): superapp"}]});
        assert_eq!(w.run(), 1);
        assert!(
            w.posted()[0].contains("state=failure")
                && w.posted()[0].contains("the pipeline errored")
        );
        let w = World::new(superapp("failed"), job("failed"));
        assert_eq!(w.run(), 1);
        assert!(w.posted()[0].contains("state=failure"));
        let w = World::new(superapp("running"), job("running"));
        w.lost.set(TRIES);
        assert_eq!(w.run(), 3);
        assert!(
            w.posted().is_empty() && w.cancelled(),
            "{:#?}",
            w.asked.borrow()
        );
        // a pipeline main's config makes no workflow of: a failure within minutes, never the hour's wait
        let w = World::new(json!({"items": []}), job("success"));
        assert_eq!(w.run(), 1);
        assert!(
            w.posted()[0].contains("no workflow in the pipeline after 5 min"),
            "{:?}",
            w.posted()
        );
        // a workflow running past the hour: cancelled, and a failure on the head
        let w = World::new(superapp("running"), job("running"));
        assert_eq!(w.run(), 1);
        assert!(
            w.cancelled() && w.posted()[0].contains("no verdict in 65 min, cancelled"),
            "{:?}",
            w.posted()
        );
    }

    /// The head check: a PR whose head moved while CircleCI ran gets no status at all; one green against main's present
    /// head is not run again, one green against an older main is.
    #[dash_test]
    fn a_moved_head_gets_no_status_and_a_green_is_reused_only_against_the_same_main() {
        let w = World::new(superapp("success"), job("success"));
        *w.heads.borrow_mut() = vec![HEAD, "cccc3333"];
        assert_eq!(w.run(), 3);
        assert!(w.posted().is_empty(), "{:#?}", w.asked.borrow());
        let mut w = World::new(superapp("success"), job("success"));
        let green = |main: &str| {
            json!([{"context": "ci/circleci: superapp", "state": "success", "target_url": "u",
                    "description": format!("pipeline 40: superapp-linux passed: aaaa1111 merged with main {main}")}])
        };
        w.statuses = green("mmmm2222");
        assert_eq!(w.run(), 0);
        assert!(
            !w.started() && w.posted().is_empty(),
            "{:#?}",
            w.asked.borrow()
        );
        let mut w = World::new(superapp("success"), job("success"));
        w.statuses = green("oooo0000");
        assert_eq!(w.run(), 0);
        assert!(w.started(), "an older main's green is run again");
    }

    /// A head whose pipeline runs (a run cut off, another session) is never started twice: its verdict is waited for and
    /// posted; the pipelines in flight and the run to start count at their worst in the reserve; under it nothing starts;
    /// a pipeline older than the wait is never read.
    #[dash_test]
    fn a_running_head_is_waited_for_not_started_twice_and_flights_count_in_the_reserve() {
        let mut w = World::new(superapp("success"), job("success"));
        w.running = vec![HEAD];
        assert_eq!(w.run(), 0, "{:#?}", w.said.borrow());
        assert!(!w.started(), "{:#?}", w.asked.borrow());
        assert!(
            w.said.borrow()[0].contains("pipeline 30 already runs for aaaa1111"),
            "{:#?}",
            w.said.borrow()
        );
        assert!(
            w.posted()[0].contains("state=success") && w.posted()[0].contains("pipeline 30"),
            "{:?}",
            w.posted()
        );
        let mut w = World::new(superapp("success"), job("success"));
        w.running = vec!["other1", "other2"];
        w.superapp_runs = json!({"items": [{"credits_used": 22_000, "duration": 600}]}).to_string();
        // 30000 - 22000 - 500 = 7500; the two in flight and this run at their worst, 3 x 65 min x 20 = 3900: 3600 stays
        assert_eq!(w.run(), 0);
        assert!(w.started(), "{:#?}", w.asked.borrow());
        let said = w.said.borrow();
        assert!(
            said[0].contains("7500 credits left")
                && said[0].contains("2 in flight and this run count 3900"),
            "{said:#?}"
        );
        drop(said);
        let mut w = World::new(superapp("success"), job("success"));
        w.running = vec!["other1", "other2"];
        w.superapp_runs = json!({"items": [{"credits_used": 24_000, "duration": 600}]}).to_string();
        assert_eq!(w.run(), 3);
        assert!(
            !w.started(),
            "5500 - 3900 is under the reserve: {:#?}",
            w.asked.borrow()
        );
        assert!(w.said.borrow()[1].contains("reserve"));
    }

    /// A red explainer's gate makes the run red and posts its failure, the CircleCI half green or not.
    #[dash_test]
    fn a_red_explainer_gate_is_a_red_run() {
        let mut w = World::new(superapp("success"), job("success"));
        w.gate = false;
        assert_eq!(w.run(), 1);
        assert!(
            w.posted_as(GATE)[0].contains("state=failure"),
            "{:?}",
            w.posted_as(GATE)
        );
    }
}
