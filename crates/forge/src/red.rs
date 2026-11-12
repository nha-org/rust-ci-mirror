//! Which step turned a check red, and whether the code did (task rust-infra-red-reruns). The apt outage of 2026-11-11 reopened
//! three met tasks and pushed PR 1442 to its hand-back cap: each red came from a step before any test, no code fault. A red is
//! infra when its job's first failed step is setup (its runner never set it up, checkout, a cache, an action, apt or the held
//! .debs, a fetch); anything else is the code's, its failing tests named from the job's log. An infra red is rerun once a later
//! run of the same job got past that step: the outage is over.
use serde_json::Value;
use std::collections::HashSet;

/// The attempt of one run whose red counts as the code's, infra or not: the first run's and the first rerun's reds are infra,
/// the second rerun's is the code's. A step that fails each time is no outage.
pub const RERUNS: u64 = 3;

/// How long an infra red waits for a later run to get past its step before its own rerun probes the outage: a quiet repo runs
/// no other job, and a red misread as infra must still reach `RERUNS` and go back to its task (/code-review of 79af9f8d5).
pub const PROBE_SECS: i64 = 3600;

/// A `gh` call, its words; the answer's stdout.
pub type Gh<'a> = &'a dyn Fn(&[&str]) -> Result<String, String>;

/// The job a red check ran as.
#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    pub repo: String,
    pub id: u64,
    pub run: u64,
    pub attempt: u64,
    pub name: String,
    pub started: String,
    /// When it ended, as GitHub writes it; empty while it runs.
    pub ended: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Red {
    pub check: String,
    /// The first failed step; `None`: none failed (its runner never set it up), or, with no `job`, its link not read.
    pub step: Option<String>,
    pub infra: bool,
    /// Its failing tests from the log, else the log's first error line.
    pub cause: Vec<String>,
    pub job: Option<Job>,
}

/// A step that runs no code of ours: GitHub's own, an action (`Run owner/name@ref`), or a command that starts as apt, the held
/// .debs, a fetch, or a lone printf to the step's outputs. Only how it starts counts: a gate of ours that fetches in its fallback
/// (explainer-gate.yml's no-elixir step) is ours (/code-review of 79af9f8d5).
/// ponytail: the held-debs step builds a crate of ours first; a compile break there reads infra until `RERUNS`.
pub fn infra_step(step: &str) -> bool {
    let cmd = step.strip_prefix("Run ").unwrap_or(step);
    let action = (cmd.split_once('@')).is_some_and(|(a, _)| a.contains('/') && !a.contains(' '));
    let setup = [
        "sudo apt-get ",
        "apt-get ",
        "git fetch ",
        "cargo run -q --locked -p dash-held-debs ",
    ];
    let output = cmd.starts_with("printf ") && cmd.contains("GITHUB_OUTPUT") && !cmd.contains("&&");
    matches!(step, "Set up job" | "Complete job")
        || step.starts_with("Post ")
        || action
        || output
        || setup.iter().any(|w| cmd.starts_with(w))
}

/// A job's JSON (`gh api repos/<repo>/actions/jobs/<id>`) read as its red; `log` asked only for a code red.
pub fn red_of(check: &str, repo: &str, job: &Value, log: impl FnOnce() -> Option<String>) -> Red {
    let step = failed_step(job).map(String::from);
    let text = |k: &str| job[k].as_str().unwrap_or_default().to_string();
    let job = Job {
        repo: repo.into(),
        id: job["id"].as_u64().unwrap_or_default(),
        run: job["run_id"].as_u64().unwrap_or_default(),
        attempt: job["run_attempt"].as_u64().unwrap_or(1),
        name: text("name"),
        started: text("started_at"),
        ended: text("completed_at"),
    };
    let infra = step.as_deref().is_none_or(infra_step) && job.attempt < RERUNS;
    let cause = if infra {
        vec![]
    } else {
        log().map(|l| causes(&l)).unwrap_or_default()
    };
    Red {
        check: check.into(),
        step,
        infra,
        cause,
        job: Some(job),
    }
}

/// A log's failing tests, as nextest prints them (« FAIL [ 0.028s] ( 5/531) crate::module test »), each once; with none, its
/// first « error » line (clippy, a build).
pub fn causes(log: &str) -> Vec<String> {
    const MARKS: [&str; 4] = ["FAIL [", "TIMEOUT [", "SIGSEGV [", "SIGABRT ["];
    let lines = log.lines().map(|l| {
        let l = crate::mirror::uncoloured(l);
        crate::mirror::untimed(&l).trim().to_string()
    });
    let lines: Vec<String> = lines.collect();
    let mut tests: Vec<String> = vec![];
    for l in &lines {
        let Some(rest) = MARKS.iter().find_map(|m| l.strip_prefix(m)) else {
            continue;
        };
        let Some((_, name)) = rest.split_once("] ") else {
            continue;
        };
        let name = match name.trim_start().strip_prefix('(') {
            Some(counted) => counted.split_once(") ").map_or(name, |(_, n)| n),
            None => name,
        };
        let name = name.trim().to_string();
        if !name.is_empty() && !tests.contains(&name) {
            tests.push(name);
        }
    }
    if tests.is_empty() {
        tests.extend(lines.into_iter().find(|l| l.starts_with("error")));
    }
    tests
}

/// `v[k]` as a list, empty when absent.
fn list<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v[k].as_array().map(Vec::as_slice).unwrap_or_default()
}

/// A job's first failed step, its name.
fn failed_step(job: &Value) -> Option<&str> {
    let steps = list(job, "steps");
    let failed = |s: &&Value| {
        let c = s["conclusion"].as_str();
        matches!(c, Some("failure" | "cancelled" | "timed_out"))
    };
    steps.iter().find(failed).and_then(|s| s["name"].as_str())
}

/// A later run's jobs (`gh api repos/<repo>/actions/runs/<id>/jobs`) hold one of `job`'s name, started after it, done, that got
/// past setup: its first failed step, if any, runs code of ours.
pub fn green_since(jobs: &Value, job: &Job) -> bool {
    let jobs = list(jobs, "jobs");
    jobs.iter().any(|j| {
        let later = j["started_at"]
            .as_str()
            .is_some_and(|at| at > job.started.as_str());
        let same =
            j["name"].as_str() == Some(job.name.as_str()) && j["id"].as_u64() != Some(job.id);
        let done = j["status"].as_str() == Some("completed");
        let past = j["conclusion"].as_str() == Some("success")
            || failed_step(j).is_some_and(|s| !infra_step(s));
        later && same && done && past
    })
}

/// A check's link: its name, a job's page or a status's, and its state (conclusion) in capitals.
pub type Link = (String, String, String);

/// A state that turns a check red, as GitHub writes a check run's conclusion or a status's state. A list, unlike lib's `red`:
/// a running check's link has no conclusion yet, which `red` would count red and rank first.
fn red_state(state: &str) -> bool {
    let reds = [
        "FAILURE",
        "CANCELLED",
        "TIMED_OUT",
        "ACTION_REQUIRED",
        "ERROR",
        "STARTUP_FAILURE",
        "STALE",
    ];
    reds.contains(&state)
}

/// Each check of `target` (a PR number or link, else a commit sha).
pub fn links(gh: Gh, target: &str) -> Result<Vec<Link>, String> {
    let pair = |v: &Value, name: &[&str], url: &[&str]| {
        let get = |ks: &[&str]| {
            ks.iter()
                .find_map(|k| v[*k].as_str())
                .unwrap_or_default()
                .to_string()
        };
        (
            get(name),
            get(url),
            get(&["conclusion", "state"]).to_uppercase(),
        )
    };
    if !crate::mirror::sha(target) {
        let view = gh(&["pr", "view", target, "--json", "statusCheckRollup"])?;
        let view: Value = serde_json::from_str(&view).map_err(|e| format!("the rollup: {e}"))?;
        let rows = list(&view, "statusCheckRollup");
        return Ok(rows
            .iter()
            .map(|r| pair(r, &["name", "context"], &["detailsUrl", "targetUrl"]))
            .collect());
    }
    let runs = gh(&[
        "api",
        &format!("repos/{{owner}}/{{repo}}/commits/{target}/check-runs?per_page=100"),
    ])?;
    let statuses = gh(&[
        "api",
        &format!("repos/{{owner}}/{{repo}}/commits/{target}/status"),
    ])?;
    let parse = |t: &str| serde_json::from_str::<Value>(t).map_err(|e| format!("the checks: {e}"));
    let (runs, statuses) = (parse(&runs)?, parse(&statuses)?);
    let runs = list(&runs, "check_runs");
    let statuses = list(&statuses, "statuses");
    let runs = runs.iter().map(|r| pair(r, &["name"], &["details_url"]));
    Ok(runs
        .chain(
            statuses
                .iter()
                .map(|s| pair(s, &["context"], &["target_url"])),
        )
        .collect())
}

/// `https://github.com/<o>/<r>/actions/runs/<run>/job/<job>`: the repo and the job id.
fn job_link(url: &str) -> Option<(String, u64)> {
    let path = url.strip_prefix("https://github.com/")?;
    let (repo, rest) = path.split_once("/actions/runs/")?;
    let id = rest.split_once("/job/")?.1.split(['?', '#', '/']).next()?;
    Some((repo.into(), id.parse().ok()?))
}

fn json(gh: Gh, path: &str) -> Result<Value, String> {
    let text = gh(&["api", path])?;
    serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))
}

fn job_red(gh: Gh, check: &str, repo: &str, id: u64) -> Result<Red, String> {
    let job = json(gh, &format!("repos/{repo}/actions/jobs/{id}"))?;
    let log = || {
        gh(&[
            "api",
            "--allow-escape-sequences",
            &format!("repos/{repo}/actions/jobs/{id}/logs"),
        ])
        .ok()
    };
    Ok(red_of(check, repo, &job, log))
}

/// The reds of `names` on `target`, each from its newest red link: its job's steps, or the mirror commit's failed check runs (a
/// check run's id is its job's). A link of no known shape is the code's, as before this read.
pub fn reds(gh: Gh, target: &str, names: &[&str]) -> Result<Vec<Red>, String> {
    let links = links(gh, target)?;
    let mut out = vec![];
    for name in names {
        // A red one first: a push run and a pull_request run share a job name, and the hook reports the name red if either is
        // (/code-review of 79af9f8d5); then the newest job id, as a superseded run's row may stand beside it.
        // A mirror row reread_mirror renamed « ci-mirror/rust (its log not posted …) » is its check's (/code-review of PR 1460).
        let mirror = crate::mirror::CONTEXT;
        let renamed = name
            .strip_prefix(mirror)
            .is_some_and(|r| r.starts_with(" ("));
        let check = if renamed { mirror } else { *name };
        let mine = links.iter().filter(|(n, _, _)| n == check);
        let url = mine
            .max_by_key(|(_, u, state)| (red_state(state), job_link(u).map_or(0, |(_, id)| id)))
            .map_or("", |(_, u, _)| u.as_str());
        if let Some((repo, id)) = job_link(url) {
            out.push(job_red(gh, name, &repo, id)?);
            continue;
        }
        let Some((repo, commit)) = crate::mirror::mirror_of(url) else {
            let unknown = Red {
                check: name.to_string(),
                step: None,
                infra: false,
                cause: vec![],
                job: None,
            };
            out.push(unknown);
            continue;
        };
        let runs = json(
            gh,
            &format!("repos/{repo}/commits/{commit}/check-runs?per_page=100"),
        )?;
        let runs = list(&runs, "check_runs");
        // Dashboard.CiMirror.state/1's rule, mirror.rs's GREEN: a completed run not success, skipped or neutral is red.
        let failed = |r: &&Value| {
            let ok = crate::mirror::GREEN.contains(&r["conclusion"].as_str().unwrap_or_default());
            r["status"].as_str() == Some("completed") && !ok
        };
        let before = out.len();
        for run in runs.iter().filter(failed) {
            let id = run["id"].as_u64().unwrap_or_default();
            out.push(job_red(
                gh,
                &format!("{name} {}", run["name"].as_str().unwrap_or("?")),
                repo,
                id,
            )?);
        }
        if out.len() == before {
            // None failed. mirror::reread, which read this red first, found it final (/code-review of 56ca68f41): no run, or the
            // run CiMirror gave up on after 24 hours, is the code's red. All done and green is its green write racing this read.
            let passed = !runs.is_empty()
                && runs
                    .iter()
                    .all(|r| r["status"].as_str() == Some("completed"));
            let step = if passed {
                "none now: its runs passed since"
            } else {
                "none failed: no run, or one CiMirror gave up on"
            };
            out.push(Red {
                check: name.to_string(),
                step: Some(step.into()),
                infra: passed,
                cause: vec![],
                job: None,
            });
        }
    }
    Ok(out)
}

/// The reds in words: each check, its failed step, infra or what failed.
pub fn words(reds: &[Red]) -> String {
    let short = |s: &str| match s.char_indices().nth(100) {
        Some((at, _)) => format!("{}…", &s[..at]),
        None => s.to_string(),
    };
    let one = |r: &Red| {
        let step = match &r.step {
            Some(s) => format!("step « {} »", short(s)),
            None if r.job.is_none() => "a step not read (no job link)".into(),
            None => "no step: its runner never set it up".into(),
        };
        let what = match (r.infra, r.cause.is_empty()) {
            (true, _) => " (infra, before any test)".to_string(),
            (false, true) => String::new(),
            (false, false) => format!(", failing: {}", r.cause.join(", ")),
        };
        let attempts = (r.job.as_ref()).filter(|j| {
            !r.infra && j.attempt >= RERUNS && r.step.as_deref().is_none_or(infra_step)
        });
        let attempts = attempts.map_or(String::new(), |j| {
            format!(" (infra, {} attempts: no rerun more)", j.attempt)
        });
        format!("{} failed at {step}{what}{attempts}", r.check)
    };
    reds.iter().map(one).collect::<Vec<_>>().join("; ")
}

/// Every red read and each infra: no verdict on the code. `checks` and `ci_wait` both judge by it (/code-review of PR 1460).
pub fn all_infra(reds: &[Red]) -> bool {
    !reds.is_empty() && reds.iter().all(|r| r.infra)
}

/// Each infra red rerun once a later run of its workflow got past its step, or once it ended before `probe` (ISO,
/// `PROBE_SECS` ago), each run once (a stale mirror status is mirror::reread's to write green). What moved, in words; none while
/// the outage lasts.
/// ponytail: up to 5 later runs of its workflow read per red at each call, an outage's few minutes; a webhook would read none.
pub fn rerun(gh: Gh, reds: &[Red], probe: &str) -> Result<Vec<String>, String> {
    let (mut done, mut failed) = (vec![], vec![]);
    let mut rerun: HashSet<(String, u64)> = HashSet::new();
    for red in reds.iter().filter(|r| r.infra) {
        let Some(job) = &red.job else { continue };
        if rerun.contains(&(job.repo.clone(), job.run)) {
            continue;
        }
        // One run's failed read or refused POST leaves the others to rerun (/code-review of PR 1460).
        match rerun_one(gh, red, job, probe) {
            Ok(Some(moved)) => {
                rerun.insert((job.repo.clone(), job.run));
                done.push(moved);
            }
            Ok(None) => {}
            Err(e) => failed.push(format!("{}: {e}", red.check)),
        }
    }
    // A failure beside a rerun rides its note, never lost (/code-review of PR 1460).
    if done.is_empty() && !failed.is_empty() {
        return Err(failed.join("; "));
    }
    Ok(done
        .into_iter()
        .chain(failed.into_iter().map(|f| format!("not rerun: {f}")))
        .collect())
}

/// A later run of `job`'s own workflow got past its step. A job with no start or a run with no workflow asks no later run, an
/// invalid query: its probe alone reruns it (/code-review of PR 1460).
fn got_past(gh: Gh, job: &Job) -> Result<bool, String> {
    if job.started.is_empty() {
        return Ok(false);
    }
    // Its own workflow's runs alone: site_perso starts runs of six others, which hold no job of its name.
    let run = json(gh, &format!("repos/{}/actions/runs/{}", job.repo, job.run))?;
    let workflow = run["workflow_id"].as_u64().unwrap_or_default();
    if workflow == 0 {
        return Ok(false);
    }
    let since = job.started.replace(':', "%3A");
    let runs = json(
        gh,
        &format!(
            "repos/{}/actions/workflows/{workflow}/runs?per_page=5&created=%3E%3D{since}",
            job.repo
        ),
    )?;
    let ids = list(&runs, "workflow_runs");
    let ids = ids
        .iter()
        .filter_map(|r| r["id"].as_u64())
        .filter(|id| *id != job.run);
    for id in ids {
        let jobs = json(gh, &format!("repos/{}/actions/runs/{id}/jobs", job.repo))?;
        if green_since(&jobs, job) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// One infra red's rerun, in words, if a later run got past its step or its probe time came; `None` while the outage lasts.
fn rerun_one(gh: Gh, red: &Red, job: &Job, probe: &str) -> Result<Option<String>, String> {
    // A red past its probe time reruns whatever later runs show: none is read (/simplify of PR 1460).
    let probed = !job.ended.is_empty() && job.ended.as_str() <= probe;
    let green = !probed && got_past(gh, job)?;
    if !green && !probed {
        return Ok(None);
    }
    gh(&[
        "api",
        "-X",
        "POST",
        &format!(
            "repos/{}/actions/runs/{}/rerun-failed-jobs",
            job.repo, job.run
        ),
    ])?;
    let step = red.step.as_deref().unwrap_or("its runner's setup");
    let why = if green {
        format!("a later run got past « {step} »")
    } else {
        format!("its red at « {step} » is over {PROBE_SECS} s old: this rerun probes it")
    };
    Ok(Some(format!(
        "{}: reran run {} of {}, as {why}",
        red.check, job.run, job.repo
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;

    fn fixture(name: &str) -> String {
        let at = format!("{}/tests/fixtures/red/{name}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(&at).unwrap_or_else(|e| panic!("{at}: {e}"))
    }

    fn value(name: &str) -> Value {
        serde_json::from_str(&fixture(name)).expect("JSON")
    }

    /// Done-when 1, the infra way: the mirror's job 130664037122 of 2026-11-11 16:44, apt failing on the runner's DNS.
    #[test]
    fn an_apt_red_from_the_mirror_s_log_is_infra_its_step_named() {
        let _budget = TestBudget::new();
        let red = red_of(
            "ci-mirror/rust test",
            "nha-org/rust-ci-mirror",
            &value("apt-job.json"),
            || Some(fixture("apt.log")),
        );
        let step = red.step.as_deref().expect("a failed step");
        assert!(step.starts_with("Run sudo apt-get update"), "{step}");
        assert!(red.infra);
        assert_eq!(red.cause, Vec::<String>::new(), "no test ran");
        let words = words(&[red]);
        assert!(
            words.contains("failed at step « Run sudo apt-get update")
                && words.ends_with("(infra, before any test)"),
            "{words}"
        );
    }

    /// A build's first error line is named whole, with or without GitHub's timestamp before it: the strip once dropped its first
    /// word (/simplify of PR 1460).
    #[test]
    fn a_build_red_names_its_first_error_line_whole() {
        let _budget = TestBudget::new();
        let cause = "error[E0432]: unresolved import `x`";
        assert_eq!(causes(&format!("Compiling\n{cause}\n")), [cause]);
        // a first word shaped like a time is kept unless it is one
        assert_eq!(crate::mirror::untimed("2Z unused"), "2Z unused");
        assert_eq!(
            causes(&format!("2026-11-11T16:44:02.1Z {cause}\n")),
            [cause]
        );
    }

    /// Done-when 1 and 3, the code way: job 130676297707 of 17:05, nextest failing one test. Its words name the step and the test.
    #[test]
    fn a_nextest_red_from_the_mirror_s_log_is_the_code_s_its_failing_test_named() {
        let _budget = TestBudget::new();
        let red = red_of(
            "ci-mirror/rust test",
            "nha-org/rust-ci-mirror",
            &value("nextest-job.json"),
            || Some(fixture("nextest.log")),
        );
        assert!(!red.infra);
        assert!(
            red.step
                .as_deref()
                .is_some_and(|s| s.contains("nextest run")),
            "{:?}",
            red.step
        );
        assert_eq!(
            red.cause,
            ["dash::compass a_page_comment_keeps_its_images_and_pdfs"]
        );
        let words = words(&[red]);
        assert!(
            words.ends_with(", failing: dash::compass a_page_comment_keeps_its_images_and_pdfs"),
            "{words}"
        );
    }

    #[test]
    fn setup_steps_are_infra_and_code_steps_are_not() {
        let _budget = TestBudget::new();
        for s in [
            "Set up job",
            "Run actions/checkout@v4",
            "Run Swatinem/rust-cache@v2",
            "Post Run actions/checkout@v4",
            "Run cargo run -q --locked -p dash-held-debs -- bubblewrap",
            "Run git fetch -q --depth=1 origin main",
            "Run printf 'os=%s\\n' \"$ImageOS\" >> \"$GITHUB_OUTPUT\"",
        ] {
            assert!(infra_step(s), "{s}");
        }
        for s in [
            "Run ci/deno.sh",
            "Run cargo check --locked -p dash --features mimalloc",
            "Run cargo run -q --locked -p dash -- ci affected FETCH_HEAD -- clippy --locked --all-targets -- -D warnings",
            "Run echo a/b@c",
            // explainer-gate.yml's no-elixir gate: ours, though its fallback fetches
            "Run { $RUNNER_TEMP/dash ci no-elixir HEAD^1 HEAD || { git fetch -q origin main; } }",
            "Run dash ci affected >> \"$GITHUB_OUTPUT\" && cargo test",
        ] {
            assert!(!infra_step(s), "{s}");
        }
    }

    /// A job its runner never set up has no failed step: infra. At attempt `RERUNS` the same red is the code's.
    #[test]
    fn a_job_with_no_failed_step_is_infra_until_its_reruns_run_out() {
        let _budget = TestBudget::new();
        let job = |attempt| serde_json::json!({"id": 1, "run_id": 2, "run_attempt": attempt, "name": "test", "steps": []});
        let red = red_of("c", "o/r", &job(1), || None);
        assert!(red.infra && red.step.is_none());
        let red = red_of("c", "o/r", &job(RERUNS), || None);
        assert!(!red.infra);
        assert!(words(&[red]).contains("attempts: no rerun more"));
    }

    /// Done-when 2's trigger: run 43064670739 of 17:05 got past apt (its nextest failed, code of ours), after the 16:44 apt red.
    #[test]
    fn a_later_run_past_the_apt_step_reads_as_green_since() {
        let _budget = TestBudget::new();
        let red = red_of(
            "c",
            "nha-org/rust-ci-mirror",
            &value("apt-job.json"),
            || None,
        );
        let job = red.job.expect("its job");
        assert!(green_since(&value("later-jobs.json"), &job));
        let mut after = job.clone();
        after.started = "2026-11-11T18:00:00Z".into();
        assert!(
            !green_since(&value("later-jobs.json"), &after),
            "a run before the red proves nothing"
        );
        let mut other = job;
        other.name = "site".into();
        assert!(
            !green_since(&value("later-jobs.json"), &other),
            "another job proves nothing"
        );
    }

    /// Done-when 2's move: the red read through gh, the later run found, its failed jobs rerun once.
    #[test]
    fn an_infra_red_is_rerun_once_a_later_run_got_past_its_step() {
        let _budget = TestBudget::new();
        let calls = std::cell::RefCell::new(vec![]);
        let gh = |args: &[&str]| -> Result<String, String> {
            let line = args.join(" ");
            calls.borrow_mut().push(line.clone());
            Ok(match line.as_str() {
                // a newer green row of the same name stands beside the red one: the red one is read
                l if l.starts_with("pr view") => serde_json::json!({"statusCheckRollup": [
                    {"name": "test", "conclusion": "FAILURE", "detailsUrl": "https://github.com/nha-org/rust-ci-mirror/actions/runs/43061229494/job/130664037122"},
                    {"name": "test", "conclusion": "SUCCESS", "detailsUrl": "https://github.com/nha-org/rust-ci-mirror/actions/runs/43061229999/job/130664039999"}
                ]}).to_string(),
                l if l.ends_with("jobs/130664037122") => fixture("apt-job.json"),
                l if l.ends_with("actions/runs/43061229494") => serde_json::json!({"workflow_id": 9}).to_string(),
                l if l.contains("actions/workflows/9/runs?") => serde_json::json!({"workflow_runs": [{"id": 43064670739u64}, {"id": 43061229494u64}]}).to_string(),
                l if l.ends_with("runs/43064670739/jobs") => fixture("later-jobs.json"),
                l if l.contains("rerun-failed-jobs") => String::new(),
                l => panic!("unexpected gh {l}"),
            })
        };
        let reds = reds(&gh, "1446", &["test"]).expect("read");
        assert!(reds.iter().all(|r| r.infra));
        let moved = rerun(&gh, &reds, "2026-11-11T16:00:00Z").expect("rerun");
        assert_eq!(
            moved,
            [
                "test: reran run 43061229494 of nha-org/rust-ci-mirror, as a later run got past « Run sudo apt-get update -qq && sudo apt-get install -y -qq bubblewrap libfontconfig1-dev poppler-utils && sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0 »"
            ]
        );
        let reruns = calls
            .borrow()
            .iter()
            .filter(|c| c.contains("rerun-failed-jobs"))
            .count();
        assert_eq!(reruns, 1);
        assert!(
            !calls.borrow().iter().any(|c| c.ends_with("/logs")),
            "an infra red reads no log"
        );
    }

    /// A quiet repo: no later run of its workflow, so the red's own rerun probes the outage once it ended `PROBE_SECS` ago, never
    /// before (/code-review of 79af9f8d5: else a misread red held its task met for good).
    #[test]
    fn an_infra_red_with_no_later_run_is_probed_after_an_hour() {
        let _budget = TestBudget::new();
        let job = serde_json::json!({"id": 5, "run_id": 6, "run_attempt": 1, "name": "test",
            "started_at": "2026-11-11T16:00:00Z", "completed_at": "2026-11-11T16:05:00Z",
            "steps": [{"name": "Run actions/checkout@v4", "conclusion": "failure"}]});
        let red = red_of("check", "o/r", &job, || None);
        let reruns = std::cell::Cell::new(0);
        let gh = |args: &[&str]| -> Result<String, String> {
            let line = args.join(" ");
            Ok(match line.as_str() {
                l if l.ends_with("actions/runs/6") => r#"{"workflow_id": 1}"#.into(),
                l if l.contains("workflows/1/runs?") => r#"{"workflow_runs": [{"id": 6}]}"#.into(),
                l if l.ends_with("runs/6/rerun-failed-jobs") => {
                    reruns.set(reruns.get() + 1);
                    String::new()
                }
                l => panic!("unexpected gh {l}"),
            })
        };
        let early = rerun(&gh, std::slice::from_ref(&red), &dash_core::iso(0)).expect("a pass");
        assert_eq!(
            (early.len(), reruns.get()),
            (0, 0),
            "within the hour: waits"
        );
        let late = rerun(&gh, &[red], "2026-11-11T16:05:00Z").expect("a pass");
        assert_eq!(reruns.get(), 1);
        assert!(late[0].contains("this rerun probes it"), "{late:?}");
        assert_eq!(dash_core::iso(1_762_879_450), "2025-11-11T16:44:10Z");
    }

    /// One run's failed read leaves the next red's rerun to go on, and a job with no start asks no later run, an invalid query:
    /// its probe alone reruns it (/code-review of PR 1460).
    #[test]
    fn one_failed_rerun_leaves_the_others_and_no_start_asks_no_later_run() {
        let _budget = TestBudget::new();
        let red = |run: u64, started: &str| {
            let job = serde_json::json!({"id": run, "run_id": run, "run_attempt": 1, "name": "test",
                "started_at": started, "completed_at": "2026-11-11T16:05:00Z", "steps": []});
            red_of("check", "o/r", &job, || None)
        };
        let gh = |args: &[&str]| -> Result<String, String> {
            let line = args.join(" ");
            match line.as_str() {
                l if l.ends_with("runs/6/rerun-failed-jobs") => Err("HTTP 403".into()),
                l if l.ends_with("runs/9/rerun-failed-jobs") => Ok(String::new()),
                l => panic!("unexpected gh {l}"),
            }
        };
        let reds = [red(6, "2026-11-11T16:00:00Z"), red(9, "")];
        // run 6's refused POST rides run 9's note
        let moved = rerun(&gh, &reds, "2026-11-11T17:05:00Z").expect("one moved");
        assert_eq!(moved.len(), 2, "{moved:?}");
        assert!(moved[0].contains("reran run 9"), "{moved:?}");
        assert!(
            moved[1].starts_with("not rerun: check: HTTP 403"),
            "{moved:?}"
        );
        let all_failed = rerun(&gh, &reds[..1], "2026-11-11T17:05:00Z");
        assert!(all_failed.is_err_and(|e| e.contains("HTTP 403")));
        // before its probe time, no start: no gh call at all
        let none = |args: &[&str]| -> Result<String, String> { panic!("gh {args:?}") };
        assert_eq!(rerun(&none, &reds[1..], &dash_core::iso(0)), Ok(vec![]));
    }

    /// A mirror red reads Dashboard.CiMirror's own rule: every run completed and success, skipped or neutral holds (infra, its
    /// status mirror::reread's to write, never rerun's); a run that ended otherwise (startup_failure) is a red read, and no run,
    /// or one still running that mirror::reread found final, is the code's (/code-review of 56ca68f41).
    #[test]
    fn a_mirror_red_is_read_on_ci_mirror_s_own_rule() {
        let _budget = TestBudget::new();
        let read = |runs: serde_json::Value| {
            let gh = move |args: &[&str]| -> Result<String, String> {
                let line = args.join(" ");
                Ok(match line.as_str() {
                    l if l.starts_with("pr view") => serde_json::json!({"statusCheckRollup": [
                        {"context": "ci-mirror/rust", "state": "FAILURE", "targetUrl": "https://github.com/o/mirror/commit/c1/checks"}
                    ]}).to_string(),
                    l if l.contains("commits/c1/check-runs") => serde_json::json!({"check_runs": runs}).to_string(),
                    l if l.contains("actions/jobs/77") => serde_json::json!({"id": 77, "run_id": 8, "run_attempt": 1, "name": "test", "steps": []}).to_string(),
                    l => panic!("unexpected gh {l}"),
                })
            };
            let got = reds(&gh, "1", &["ci-mirror/rust"]).expect("read");
            // the row reread_mirror renamed reads the same link (/code-review of PR 1460)
            let renamed = reds(
                &gh,
                "1",
                &["ci-mirror/rust (its log not posted on the PR: 502)"],
            );
            assert_eq!(
                renamed
                    .expect("read")
                    .iter()
                    .map(|r| r.infra)
                    .collect::<Vec<_>>(),
                got.iter().map(|r| r.infra).collect::<Vec<_>>()
            );
            got
        };
        let passed = read(
            serde_json::json!([{"id": 1, "status": "completed", "conclusion": "success"},
            {"id": 2, "status": "completed", "conclusion": "skipped"}]),
        );
        assert!(passed[0].infra, "{passed:?}");
        let no_write =
            |args: &[&str]| -> Result<String, String> { panic!("rerun called gh {args:?}") };
        assert_eq!(rerun(&no_write, &passed, &dash_core::iso(0)), Ok(vec![]));
        assert_eq!(
            words(&passed),
            "ci-mirror/rust failed at step « none now: its runs passed since » (infra, before any test)"
        );
        let odd = read(
            serde_json::json!([{"id": 77, "name": "test", "status": "completed", "conclusion": "startup_failure"}]),
        );
        assert_eq!(odd.len(), 1, "{odd:?}");
        assert_eq!(odd[0].check, "ci-mirror/rust test");
        let running =
            read(serde_json::json!([{"id": 1, "status": "in_progress", "conclusion": null}]));
        assert!(!running[0].infra, "{running:?}");
        let none = read(serde_json::json!([]));
        assert_eq!(
            (none[0].infra, words(&none)),
            (
                false,
                "ci-mirror/rust failed at step « none failed: no run, or one CiMirror gave up on »"
                    .into()
            )
        );
    }

    /// A red whose link was not read says so, never « its runner never set it up ».
    #[test]
    fn a_red_with_no_link_is_said_unread_and_the_code_s() {
        let _budget = TestBudget::new();
        let gh = |_: &[&str]| -> Result<String, String> {
            Ok(serde_json::json!({"statusCheckRollup": [{"context": "ci-mirror/rust", "state": "EXPECTED"}]}).to_string())
        };
        let got = reds(&gh, "1", &["ci-mirror/rust"]).expect("read");
        assert!(!got[0].infra);
        assert_eq!(
            words(&got),
            "ci-mirror/rust failed at a step not read (no job link)"
        );
    }
}
