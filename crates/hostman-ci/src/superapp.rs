//! The superapp's CI on CircleCI while no Hostman box is up (task superapp-ci-circleci; @nha, 2026-11-18: « about CI: add
//! monitoring ? datadog ? if not lmk we find a free CI provider. », then « yes » to CircleCI). superapp.yml's job asks for
//! `[ci]`, so with no box it waits queued, a pending check no merge gets past. Each minute with no box up, `pass` hands each
//! queued pull request run to CircleCI: `.circleci/config.yml`'s `superapp` job, the same cargo lines, started through the
//! API with `superapp: true` on the PR's merge ref; the head holds `CONTEXT` pending, which CircleCI's verdict replaces; the
//! GitHub run is cancelled, and dash-forge's checks read that cancel as given way to `CONTEXT`. A push to main stays
//! queued for the box, as the older router kept main off CircleCI.
//! ponytail: a head whose CircleCI run failed is kept: a GitHub re-run while no box is up is cancelled again; rerun it from
//! CircleCI's page, or push.
use serde_json::{Value as Json, json};
use std::path::Path;
use std::process::{Command, Stdio};

/// The status CircleCI posts for the `superapp` job, the one `pass` holds pending first.
pub const CONTEXT: &str = "ci/circleci: superapp";
const REPO: &str = "nha-org/site_perso";
const SLUG: &str = "gh/nha-org/site_perso";
/// A CircleCI workflow still running (API v2's states): a pipeline of an older head's is cancelled.
const LIVE: [&str; 3] = ["running", "on_hold", "failing"];

/// gh with these arguments under the host's login: its output.
pub type Gh<'a> = &'a dyn Fn(&[&str]) -> Result<String, String>;
/// CircleCI's API v2 at this path, a POST of the body when one is given: the answer's body (`api`).
pub type Circle<'a> = &'a dyn Fn(&str, Option<&str>) -> Result<String, String>;

fn parsed(text: &str) -> Result<Json, String> {
    serde_json::from_str(text).map_err(|e| {
        format!(
            "not JSON ({e}): {}",
            text.chars().take(120).collect::<String>()
        )
    })
}

/// Each queued pull request run of superapp.yml handed to CircleCI, a line each: what was done, or why not.
pub fn pass(gh: Gh, circle: Circle) -> Result<Vec<String>, String> {
    let path = format!(
        "repos/{REPO}/actions/workflows/superapp.yml/runs?status=queued&event=pull_request&per_page=50"
    );
    let runs = parsed(&gh(&["api", &path])?)?;
    let runs = runs["workflow_runs"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    // ponytail: five a minute, so one tick never holds the boxes' own reconcile for long; the rest at the next minutes
    Ok(runs
        .iter()
        .take(5)
        .map(|r| {
            let id = r["id"].as_u64().unwrap_or_default();
            one(gh, circle, r).unwrap_or_else(|e| format!("run {id}: {e}"))
        })
        .collect())
}

/// One queued run: a PR closed or moved on has its run cancelled; else its head's pipeline is started (or one already
/// started kept) and the run cancelled.
fn one(gh: Gh, circle: Circle, run: &Json) -> Result<String, String> {
    let id = run["id"].as_u64().ok_or("no run id")?;
    let (sha, branch) = (
        run["head_sha"].as_str().unwrap_or_default(),
        run["head_branch"].as_str().unwrap_or_default(),
    );
    let cancel = || {
        gh(&[
            "api",
            "-X",
            "POST",
            &format!("repos/{REPO}/actions/runs/{id}/cancel"),
        ])
        .map(drop)
    };
    // GitHub names an open PR alone: a merged one's run lists none (ten such queued on 2026-11-18, their PRs merged)
    // ponytail: a fork's PR lists none either; this private repository has no fork
    let Some(pr) = run["pull_requests"][0]["number"].as_u64() else {
        cancel()?;
        return Ok(format!(
            "run {id}: no open pull request heads it, cancelled"
        ));
    };
    let p = parsed(&gh(&["api", &format!("repos/{REPO}/pulls/{pr}")])?)?;
    if p["state"] != "open" || p["head"]["sha"] != sha {
        cancel()?;
        return Ok(format!(
            "run {id}: PR {pr} closed or moved past {}, cancelled",
            short(sha)
        ));
    }
    // the merge commit GitHub made for this head, so the code tested is the one the verdict lands on; its ref while none
    let tested = (p["merge_commit_sha"].as_str())
        .filter(|m| !m.is_empty())
        .map_or(format!("refs/pull/{pr}/merge"), String::from);
    let (n, said) = match pipeline(circle, branch, sha)? {
        Some(n) => (n, format!("PR {pr}: pipeline {n} of {} kept", short(sha))),
        None => {
            let n = trigger(circle, branch, &tested)?;
            let said = format!(
                "PR {pr}: pipeline {n} started for {}: {}",
                short(sha),
                url(n)
            );
            (n, said)
        }
    };
    // pending until CircleCI's own status, posted only where the head holds none of `CONTEXT`: never over a verdict, and
    // again on a kept pipeline whose first post was lost, or `checks::handed` would read the cancel as red
    let statuses = parsed(&gh(&[
        "api",
        &format!("repos/{REPO}/commits/{sha}/statuses?per_page=100"),
    ])?)?;
    if !(statuses.as_array().into_iter().flatten()).any(|s| s["context"] == CONTEXT) {
        gh(&[
            "api",
            "-X",
            "POST",
            &format!("repos/{REPO}/statuses/{sha}"),
            "-f",
            "state=pending",
            "-f",
            &format!("context={CONTEXT}"),
            "-f",
            &format!("description=CircleCI pipeline {n} started by the router"),
            "-f",
            &format!("target_url={}", url(n)),
        ])?;
    }
    cancel()?;
    Ok(format!("{said}, run {id} cancelled"))
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(8)]
}

fn url(n: u64) -> String {
    format!("https://app.circleci.com/pipelines/{SLUG}/{n}")
}

/// The branch's routed pipeline of `sha`, if one was started; the routed pipelines of its older heads have their running
/// workflows cancelled, as superapp.yml's concurrency cancels an older run. CircleCI's first page, 20 pipelines.
fn pipeline(circle: Circle, branch: &str, sha: &str) -> Result<Option<u64>, String> {
    let listed = parsed(&circle(
        &format!("project/{SLUG}/pipeline?branch={}", encoded(branch)),
        None,
    )?)?;
    let items = listed["items"].as_array().cloned().unwrap_or_default();
    let routed: Vec<&Json> = (items.iter())
        .filter(|p| p["trigger"]["type"] == "api")
        .collect();
    if let Some(p) = routed.iter().find(|p| p["vcs"]["revision"] == sha) {
        // one that errored ran nothing (its config refused it): said each minute, never started again for this head
        if p["state"] == "errored" {
            return Err(format!(
                "pipeline {} errored, {branch}'s .circleci/config.yml refused it: merge main into it",
                p["number"]
            ));
        }
        return Ok(p["number"].as_u64());
    }
    // best effort: a cancel refused costs credits, never this head's hand-over
    for p in routed {
        let id = p["id"].as_str().unwrap_or_default();
        let Ok(flows) = circle(&format!("pipeline/{id}/workflow"), None) else {
            continue;
        };
        for w in (parsed(&flows)?["items"].as_array().cloned())
            .into_iter()
            .flatten()
        {
            if LIVE.iter().any(|s| w["status"] == *s) {
                let wid = w["id"].as_str().unwrap_or_default();
                let _ = circle(&format!("workflow/{wid}/cancel"), Some("{}"));
            }
        }
    }
    Ok(None)
}

/// The superapp job's pipeline on `branch`, testing `tested` (a PR's merge commit, as superapp.yml tests a PR merged into
/// its base): its number once CircleCI read the config. `routed` too: the `dashboard` workflow runs only with it, and its
/// `check` job is filtered out by `superapp`. A config that refuses `superapp` (a branch from before this job, which reads
/// its own .circleci/config.yml) errors: the run is left queued and the line says to merge main.
fn trigger(circle: Circle, branch: &str, tested: &str) -> Result<u64, String> {
    let body =
        json!({"branch": branch, "parameters": {"routed": true, "superapp": true, "ref": tested}});
    let got = parsed(&circle(
        &format!("project/{SLUG}/pipeline"),
        Some(&body.to_string()),
    )?)?;
    let n = (got["number"].as_u64()).ok_or(format!("CircleCI gave no pipeline number: {got}"))?;
    let id = got["id"].as_str().unwrap_or_default();
    // CircleCI reads the config within seconds; past 20 s still pending, the pipeline is taken, its statuses to come
    for _ in 0..10 {
        let p = parsed(&circle(&format!("pipeline/{id}"), None)?)?;
        match p["state"].as_str() {
            Some("errored") => {
                let why = p["errors"][0]["message"]
                    .as_str()
                    .unwrap_or("no reason given");
                return Err(format!(
                    "pipeline {n} errored, {branch}'s .circleci/config.yml refused it (merge main into it): {}",
                    why.chars().take(160).collect::<String>()
                ));
            }
            Some("created") => break,
            _ => std::thread::sleep(std::time::Duration::from_secs(2)),
        }
    }
    Ok(n)
}

/// A query value, its reserved characters percent-encoded.
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

/// CircleCI's API with the token bin/secret holds under `circleci` in `repo`, read at the first call: the token goes in
/// curl's environment alone, never its command line or a log.
/// ponytail: std::process as comments' hot.rs runs git, bounded by curl's own --max-time; a plugin crate takes no
/// dash-process (dash-gate's `a_plugin_takes_dash_sdk_alone_held_per_crate`).
pub fn api(repo: &Path) -> impl Fn(&str, Option<&str>) -> Result<String, String> + '_ {
    let token = std::cell::OnceCell::new();
    move |path, body| {
        // his circleci.env names it CIRCLE_API_TOKEN, bin/secret's renewal CIRCLE_TOKEN: either reads, as the older app's
        let key = token.get_or_init(|| {
            secret(repo, "CIRCLE_API_TOKEN").or_else(|_| secret(repo, "CIRCLE_TOKEN"))
        });
        curl(key.as_deref().map_err(Clone::clone)?, path, body)
    }
}

/// bin/secret's `circleci` value under `var`, waited for 15 s at most: the recurring jobs' one thread never hangs on it.
fn secret(repo: &Path, var: &str) -> Result<String, String> {
    let mut child = Command::new(repo.join("bin/secret"))
        .args(["get", "circleci", var])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("bin/secret: {e}"))?;
    let until = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while child.try_wait().map_err(|e| e.to_string())?.is_none() {
        if std::time::Instant::now() > until {
            let _ = child.kill();
            let _ = child.wait();
            return Err("bin/secret get circleci: no answer in 15 s".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let o = child.wait_with_output().map_err(|e| e.to_string())?;
    let value = String::from_utf8_lossy(&o.stdout).trim().to_string();
    match o.status.success() && !value.is_empty() {
        true => Ok(value),
        false => Err(format!("bin/secret get circleci {var}: none")),
    }
}

fn curl(key: &str, path: &str, body: Option<&str>) -> Result<String, String> {
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
    let ran = Command::new("curl")
        .args(&args)
        .env("KEY", key)
        .stdin(Stdio::null())
        .output();
    let o = ran.map_err(|e| format!("curl: {e}"))?;
    let text = String::from_utf8_lossy(&o.stdout);
    let (said, code) = text
        .trim_end()
        .rsplit_once('\n')
        .unwrap_or(("", text.trim()));
    match code.parse::<u16>() {
        Ok(200..=299) => Ok(said.to_string()),
        Ok(code) => Err(format!(
            "CircleCI {path}: HTTP {code}: {}",
            said.chars().take(160).collect::<String>()
        )),
        Err(_) => Err(format!(
            "CircleCI {path}: no answer: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;
    use std::cell::RefCell;

    /// Done-when of task superapp-ci-circleci: an open PR's queued run starts its head's pipeline, holds the head pending and
    /// is cancelled, its older head's pipeline stopped; a run of a merged PR, named or not, is cancelled alone; a head whose pipeline was
    /// started is kept, no second one, no pending over its verdict; a head whose
    /// pipeline errored (a branch from before the job) is said and left queued, never started again.
    #[dash_test]
    fn a_queued_superapp_run_goes_to_circleci_once_and_its_github_run_is_cancelled() {
        let said = RefCell::new(vec![]);
        let runs = json!({"workflow_runs": [
            {"id": 1, "head_sha": "aaaa1111bbbb", "head_branch": "task/x", "pull_requests": [{"number": 5}]},
            {"id": 2, "head_sha": "cccc", "head_branch": "task/merged", "pull_requests": [{"number": 6}]},
            {"id": 3, "head_sha": "dddd", "head_branch": "task/kept", "pull_requests": [{"number": 7}]},
            {"id": 4, "head_sha": "eeee", "head_branch": "task/merged-too", "pull_requests": []},
            {"id": 5, "head_sha": "ffff", "head_branch": "task/old", "pull_requests": [{"number": 8}]},
        ]});
        let open = |sha: &str| json!({"state": "open", "head": {"sha": sha}}).to_string();
        let gh = |args: &[&str]| -> Result<String, String> {
            said.borrow_mut().push(format!("gh {}", args.join(" ")));
            Ok(match args.last().copied().unwrap_or_default() {
                p if p.contains("/runs?status=queued") => runs.to_string(),
                p if p.ends_with("/pulls/5") => json!({"state": "open", "head": {"sha": "aaaa1111bbbb"}, "merge_commit_sha": "m5"}).to_string(),
                p if p.ends_with("/pulls/6") => json!({"state": "closed", "head": {"sha": "cccc"}}).to_string(),
                p if p.ends_with("/pulls/7") => open("dddd"),
                p if p.ends_with("/pulls/8") => open("ffff"),
                // the kept head holds CircleCI's verdict already, the new one none
                p if p.contains("commits/dddd/statuses") => json!([{"context": CONTEXT, "state": "success"}]).to_string(),
                p if p.contains("/statuses?") => "[]".into(),
                _ => String::new(),
            })
        };
        let circle = |path: &str, body: Option<&str>| -> Result<String, String> {
            said.borrow_mut()
                .push(format!("circle {path} {}", body.unwrap_or("")));
            let api = |id: &str, n: u64, sha: &str, state: &str| json!({"id": id, "number": n, "trigger": {"type": "api"}, "vcs": {"revision": sha}, "state": state});
            Ok(match path {
                p if p.ends_with("branch=task%2Fx") => json!({"items": [
                    api("p-old", 40, "old", "created"),
                    {"id": "p-hook", "number": 41, "trigger": {"type": "webhook"}, "vcs": {"revision": "aaaa1111bbbb"}},
                ]})
                .to_string(),
                p if p.ends_with("branch=task%2Fkept") => json!({"items": [api("p7", 44, "dddd", "created")]}).to_string(),
                p if p.ends_with("branch=task%2Fold") => json!({"items": [api("p8", 46, "ffff", "errored")]}).to_string(),
                "pipeline/p-old/workflow" => json!({"items": [{"id": "w1", "status": "running"}, {"id": "w2", "status": "success"}]}).to_string(),
                "pipeline/p-new" => json!({"state": "created"}).to_string(),
                p if p.ends_with("/pipeline") => json!({"number": 42, "id": "p-new"}).to_string(),
                _ => "{}".into(),
            })
        };
        let lines = pass(&gh, &circle).expect("a pass");
        assert_eq!(
            lines,
            [
                "PR 5: pipeline 42 started for aaaa1111: https://app.circleci.com/pipelines/gh/nha-org/site_perso/42, run 1 cancelled",
                "run 2: PR 6 closed or moved past cccc, cancelled",
                "PR 7: pipeline 44 of dddd kept, run 3 cancelled",
                "run 4: no open pull request heads it, cancelled",
                "run 5: pipeline 46 errored, task/old's .circleci/config.yml refused it: merge main into it",
            ]
        );
        let said = said.into_inner();
        let has = |s: &str| said.iter().filter(|l| l.contains(s)).count();
        assert_eq!(
            has("circle workflow/w1/cancel"),
            1,
            "the older head's running workflow stopped: {said:#?}"
        );
        assert_eq!(has("workflow/w2/cancel"), 0, "an ended one left");
        let trigger = r#"circle project/gh/nha-org/site_perso/pipeline {"branch":"task/x","parameters":{"ref":"m5","routed":true,"superapp":true}}"#;
        assert_eq!(has(trigger), 1, "the merge commit tested: {said:#?}");
        assert_eq!(
            has("statuses/aaaa1111bbbb -f state=pending -f context=ci/circleci: superapp"),
            1
        );
        assert_eq!(
            has("-f state=pending"),
            1,
            "no pending over a kept pipeline's verdict"
        );
        for run in 1..=4 {
            assert_eq!(has(&format!("actions/runs/{run}/cancel")), 1);
        }
        assert_eq!(
            has("actions/runs/5/cancel"),
            0,
            "an errored head's run stays queued"
        );
        assert_eq!(
            has(r#""branch":"task/old""#),
            0,
            "and is never started again"
        );
    }

    /// A refused trigger leaves the run queued, so the next minute tries again, and says why.
    #[dash_test]
    fn a_refused_trigger_leaves_the_run_queued() {
        let cancels = RefCell::new(0);
        let gh = |args: &[&str]| -> Result<String, String> {
            let p = args.last().copied().unwrap_or_default();
            *cancels.borrow_mut() += usize::from(p.ends_with("/cancel"));
            Ok(match p {
                p if p.contains("status=queued") => json!({"workflow_runs": [{"id": 1, "head_sha": "ab", "head_branch": "b", "pull_requests": [{"number": 5}]}]}).to_string(),
                _ => json!({"state": "open", "head": {"sha": "ab"}}).to_string(),
            })
        };
        let circle = |path: &str, body: Option<&str>| -> Result<String, String> {
            match body {
                Some(_) => Err(format!("CircleCI {path}: HTTP 403: out of credits")),
                None => Ok(json!({"items": []}).to_string()),
            }
        };
        let lines = pass(&gh, &circle).expect("a pass");
        assert_eq!(
            lines,
            ["run 1: CircleCI project/gh/nha-org/site_perso/pipeline: HTTP 403: out of credits"]
        );
        assert_eq!(cancels.into_inner(), 0);
    }
}
