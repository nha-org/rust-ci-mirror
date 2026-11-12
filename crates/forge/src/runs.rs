//! `dash ci list`: where a repository's CI jobs ran over whole UTC days, and how long each waited for a runner (task
//! rust-ci-off-mac; @nha, 2026-11-11: « that and the CI runner should probably go to the VPS »). A job is the Mac's when its
//! runs-on names `self-hosted` (every Mac runner carries it, the VPS's was registered with --no-default-labels), the VPS's
//! when it names `site-perso-vps`, else a hosted machine's. Why a job took the Mac is its label: the gate runner, the router's
//! `site-perso` pool, or replicator's. The wait is the job's start less its creation.
//!
//! GitHub lists at most 1000 runs a query, so each day is its own; a run's jobs are one call each. A finished run never
//! changes, so its jobs are kept in a cache file, one JSON line a run, and a later read asks only for new runs. The calls
//! stop while GitHub's hourly budget is under [`FLOOR`], and the report says how many runs went unread. A green run's time,
//! its attempt's start to its last update (waits included), comes from the day listings, at no call of its own; only a
//! first attempt is timed, so its place and its time are one attempt's. A run's place is every place its jobs ran, `mac+vps`
//! for a Mac setup job routing its work to the VPS.
// ponytail: updated_at also moves on a later touch of a finished run (its logs deleted), so a p90 can overstate; the jobs'
// completed_at bounds it, at a cache format change and a re-read of every run.
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::Path;

/// The calls left in GitHub's hour under which no more jobs are read: the orchestrator and every session share them.
pub const FLOOR: i64 = 1500;
/// Run reads in flight at once.
const LANES: usize = 8;

/// One job of a run, as `GET /repos/{slug}/actions/runs/{id}/jobs` gives it.
#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    pub workflow: String,
    pub name: String,
    pub labels: Vec<String>,
    /// Unix seconds; a job never started (skipped, cancelled while queued) has no start.
    pub created: i64,
    pub started: Option<i64>,
}

/// Where a job ran, by the labels it asked for.
pub fn class(labels: &[String]) -> &'static str {
    if labels.iter().any(|l| l == "self-hosted") {
        "mac"
    } else if labels.iter().any(|l| l == "site-perso-vps") {
        "vps"
    } else {
        "hosted"
    }
}

/// Why a Mac job took the Mac: the one runner its labels name.
fn why(labels: &[String]) -> String {
    match labels.iter().find(|l| *l != "self-hosted") {
        Some(l) if l == "site-perso-gate" => {
            "gate: the gate runner (site-perso-gate), fixed in its workflow".into()
        }
        Some(l) if l == "site-perso" => "route: the router's Mac pool (site-perso)".into(),
        Some(l) if l == "replicator" => "replicator: its own Mac runner".into(),
        Some(l) => format!("{l}: a Mac runner"),
        None => "self-hosted alone".into(),
    }
}

/// Unix seconds of GitHub's `2026-11-08T15:22:34Z`.
pub fn unix(iso: &str) -> Option<i64> {
    let day = dash_core::days(iso.get(..10)?)?;
    let t = iso.get(10..19)?.strip_prefix('T')?;
    let field = |r: std::ops::Range<usize>| t.get(r)?.parse::<i64>().ok();
    Some(day * 86400 + field(0..2)? * 3600 + field(3..5)? * 60 + field(6..8)?)
}

fn median_p90(waits: &mut [i64]) -> (i64, i64) {
    waits.sort_unstable();
    let at = |q: usize| waits.get((waits.len() * q / 100).min(waits.len().saturating_sub(1)));
    (*at(50).unwrap_or(&0), *at(90).unwrap_or(&0))
}

/// The report on jobs that started, then the green runs' times from `took` (workflow and place, seconds); `unread`
/// runs whose jobs were not read are named, never left out silently.
pub fn report(
    slug: &str,
    from: &str,
    days: i64,
    runs: usize,
    unread: usize,
    jobs: &[Job],
    took: &[(String, i64)],
) -> String {
    let ran: Vec<&Job> = jobs.iter().filter(|j| j.started.is_some()).collect();
    let wait = |j: &Job| j.started.unwrap_or(j.created) - j.created;
    let line = |label: &str, js: &[&Job]| {
        let mut w: Vec<i64> = js.iter().map(|j| wait(j)).collect();
        let (m, p) = median_p90(&mut w);
        format!(
            "  {label:<58} {:>5} jobs, wait median {m} s, p90 {p} s\n",
            js.len()
        )
    };
    let to = dash_core::date(dash_core::days(from).unwrap_or(0) + days - 1);
    let mut said = format!(
        "ci list {slug}, {from} to {to} (UTC, {days} days): {runs} runs, {} jobs ran\n",
        ran.len()
    );
    if unread > 0 {
        said += &format!(
            "  {unread} runs unread (GitHub's hourly budget under {FLOOR}, or not finished): run it again later\n"
        );
    }
    said += "where:\n";
    for place in ["mac", "vps", "hosted"] {
        let js: Vec<&Job> = ran
            .iter()
            .copied()
            .filter(|j| class(&j.labels) == place)
            .collect();
        said += &line(place, &js);
    }
    let mut whys: BTreeMap<String, Vec<&Job>> = BTreeMap::new();
    for j in ran.iter().copied().filter(|j| class(&j.labels) == "mac") {
        whys.entry(why(&j.labels)).or_default().push(j);
    }
    said += "on the Mac, why:\n";
    if whys.is_empty() {
        said += "  none: no job ran on the Mac\n";
    }
    for (w, js) in &whys {
        said += &line(w, js);
    }
    let mut by: BTreeMap<(String, String, &'static str), Vec<&Job>> = BTreeMap::new();
    for j in ran.iter().copied() {
        by.entry((j.workflow.clone(), j.name.clone(), class(&j.labels)))
            .or_default()
            .push(j);
    }
    said += "by workflow, job and place:\n";
    for ((w, n, place), js) in &by {
        said += &line(&format!("{w} {n} ({place})"), js);
    }
    let mut times: BTreeMap<&str, Vec<i64>> = BTreeMap::new();
    for (w, t) in took {
        times.entry(w).or_default().push(*t);
    }
    said += "green runs by workflow, attempt start to done (waits included):\n";
    if took.is_empty() {
        said += "  none: no green run\n";
    }
    for (w, ts) in &mut times {
        let (m, p) = median_p90(ts);
        said += &format!("  {w:<58} {:>5} runs, median {m} s, p90 {p} s\n", ts.len());
    }
    said
}

/// The jobs of one run's `jobs` answer, or of its pages as `gh api --paginate --slurp` gives them.
pub fn jobs_of(workflow: &str, answer: &Value) -> Vec<Job> {
    let pages = match answer {
        Value::Array(pages) => pages.as_slice(),
        page => std::slice::from_ref(page),
    };
    pages
        .iter()
        .filter_map(|p| p.get("jobs").and_then(Value::as_array))
        .flatten()
        .filter_map(|j| {
            Some(Job {
                workflow: workflow.to_string(),
                name: j.get("name")?.as_str()?.to_string(),
                labels: j
                    .get("labels")?
                    .as_array()?
                    .iter()
                    .filter_map(|l| Some(l.as_str()?.to_string()))
                    .collect(),
                created: unix(j.get("created_at")?.as_str()?)?,
                started: j
                    .get("started_at")
                    .and_then(Value::as_str)
                    .and_then(unix)
                    .filter(|_| j.get("conclusion").and_then(Value::as_str) != Some("skipped")),
            })
        })
        .collect()
}

fn line_of(run: u64, attempt: u64, jobs: &[Job]) -> String {
    let jobs: Vec<Value> = jobs
        .iter()
        .map(|j| json!({"workflow": j.workflow, "name": j.name, "labels": j.labels, "created": j.created, "started": j.started}))
        .collect();
    json!({"run": run, "attempt": attempt, "jobs": jobs}).to_string()
}

/// Each run's newest line: a re-run's attempt, read later, is appended after its first.
fn read_cache(text: &str) -> BTreeMap<u64, (u64, Vec<Job>)> {
    let mut runs = BTreeMap::new();
    for v in text
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
    {
        let Some(run) = v.get("run").and_then(Value::as_u64) else {
            continue;
        };
        let jobs = v
            .get("jobs")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let jobs = jobs
            .iter()
            .filter_map(|j| {
                Some(Job {
                    workflow: j.get("workflow")?.as_str()?.to_string(),
                    name: j.get("name")?.as_str()?.to_string(),
                    labels: j
                        .get("labels")?
                        .as_array()?
                        .iter()
                        .filter_map(|l| Some(l.as_str()?.to_string()))
                        .collect(),
                    created: j.get("created")?.as_i64()?,
                    started: j.get("started").and_then(Value::as_i64),
                })
            })
            .collect();
        let attempt = v.get("attempt").and_then(Value::as_u64).unwrap_or(1);
        runs.insert(run, (attempt, jobs));
    }
    runs
}

/// `dash ci list`: the runs of each day from `from` for `days`, their jobs from `cache` or through `ask` (`gh` and its
/// words), the jobs of each finished run read added to `cache`.
pub fn list(
    ask: &(dyn Fn(&[&str]) -> Result<String, String> + Sync),
    slug: &str,
    from: &str,
    days: i64,
    cache: &Path,
) -> Result<String, String> {
    let first = dash_core::days(from).ok_or(format!("--from {from}: no such day"))?;
    // opened before any call, so a refused write spends none; each run's line is appended as soon as it is read, so a cut
    // (a bound, a kill) keeps every call already paid for
    if let Some(dir) = cache.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let file = std::sync::Mutex::new(
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(cache)
            .map_err(|e| format!("{}: {e}", cache.display()))?,
    );
    let mut runs: Vec<(u64, String, bool, u64)> = vec![];
    // by run id: a listing walked while runs are created can give a run on two pages
    let mut took: BTreeMap<u64, (String, i64)> = BTreeMap::new();
    for d in first..first + days {
        let path = format!(
            "repos/{slug}/actions/runs?created={}&per_page=100",
            dash_core::date(d)
        );
        let jq = ".workflow_runs[] | [.id, .path, .status, .run_attempt, .conclusion, .run_started_at, .updated_at] | @tsv";
        for row in ask(&["api", "--paginate", &path, "--jq", jq])?.lines() {
            let f: Vec<&str> = row.split('\t').collect();
            if let [id, path, status, attempt, conclusion, start, end] = f[..]
                && let Ok(id) = id.parse()
            {
                let workflow = path.rsplit('/').next().unwrap_or(path).to_string();
                if let (Some(a), Some(b), "success", "1") =
                    (unix(start), unix(end), conclusion, attempt)
                {
                    took.insert(id, (workflow.clone(), b - a));
                }
                runs.push((
                    id,
                    workflow,
                    status == "completed",
                    attempt.parse().unwrap_or(1),
                ));
            }
        }
    }
    let mut known = read_cache(&std::fs::read_to_string(cache).unwrap_or_default());
    let budget = ask(&["api", "rate_limit", "--jq", ".resources.core.remaining"])?
        .trim()
        .parse::<i64>()
        .unwrap_or(0);
    // the runs to read, finished ones only and a re-run again, as many as the budget allows over its floor
    let todo: Vec<&(u64, String, bool, u64)> = runs
        .iter()
        .filter(|(id, _, done, attempt)| *done && known.get(id).map(|(a, _)| a) != Some(attempt))
        .take((budget - FLOOR).max(0) as usize)
        .collect();
    let read: Vec<(u64, (u64, Vec<Job>))> = std::thread::scope(|s| {
        let lanes: Vec<_> = todo
            .chunks(todo.len().div_ceil(LANES).max(1))
            .map(|chunk| {
                let file = &file;
                s.spawn(move || {
                    chunk
                        .iter()
                        .filter_map(|(id, workflow, _, attempt)| {
                            // every attempt's jobs, each page: a re-run's first attempt ran somewhere too
                            let path = format!(
                                "repos/{slug}/actions/runs/{id}/jobs?filter=all&per_page=100"
                            );
                            let answer: Value = serde_json::from_str(
                                &ask(&["api", "--paginate", "--slurp", &path]).ok()?,
                            )
                            .ok()?;
                            let jobs = jobs_of(workflow, &answer);
                            let mut f = file.lock().ok()?;
                            writeln!(f, "{}", line_of(*id, *attempt, &jobs)).ok()?;
                            Some((*id, (*attempt, jobs)))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        lanes
            .into_iter()
            .flat_map(|l| l.join().unwrap_or_default())
            .collect()
    });
    known.extend(read);
    let ids: BTreeSet<u64> = runs.iter().map(|(id, ..)| *id).collect();
    let jobs: Vec<Job> = ids
        .iter()
        .filter_map(|id| known.get(id))
        .flat_map(|(_, jobs)| jobs)
        .cloned()
        .collect();
    let unread = ids.iter().filter(|id| !known.contains_key(id)).count();
    // a run is placed by its jobs' labels: the Mac's gate before the move, the VPS's after
    let took: Vec<(String, i64)> = took
        .into_iter()
        .map(|(id, (w, t))| {
            let places: BTreeSet<&str> = known
                .get(&id)
                .map(|(_, js)| {
                    js.iter()
                        .filter(|j| j.started.is_some())
                        .map(|j| class(&j.labels))
                        .collect()
                })
                .unwrap_or_default();
            let place = if places.is_empty() {
                "unread".to_string()
            } else {
                places.into_iter().collect::<Vec<_>>().join("+")
            };
            (format!("{w} ({place})"), t)
        })
        .collect();
    Ok(report(slug, from, days, ids.len(), unread, &jobs, &took))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;
    use std::sync::Mutex;

    fn job(workflow: &str, name: &str, labels: &[&str], wait: i64) -> Job {
        Job {
            workflow: workflow.into(),
            name: name.into(),
            labels: labels.iter().map(|l| l.to_string()).collect(),
            created: 1_000,
            started: Some(1_000 + wait),
        }
    }

    /// Done-when 1 and 5 of rust-ci-off-mac: each job is placed by its labels, the Mac's by why, waits quoted; a week with no
    /// Mac job says so.
    #[test]
    fn jobs_are_placed_by_their_labels_and_the_mac_ones_by_why() {
        let _budget = TestBudget::new();
        let jobs = vec![
            job(
                "explainer-gate.yml",
                "gate",
                &["self-hosted", "site-perso-gate"],
                10,
            ),
            job(
                "explainer-gate.yml",
                "gate",
                &["self-hosted", "site-perso-gate"],
                30,
            ),
            job("site.yml", "test", &["self-hosted", "site-perso"], 600),
            job("explainer-gate.yml", "gate", &["site-perso-vps"], 4),
            job(
                "ci-usage.yml",
                "usage",
                &["blacksmith-2vcpu-ubuntu-2404"],
                6,
            ),
            Job {
                started: None,
                ..job("site.yml", "test", &["self-hosted", "site-perso"], 0)
            },
        ];
        let took: Vec<(String, i64)> = [50, 40, 227, 63]
            .iter()
            .map(|t| ("explainer-gate.yml (mac)".to_string(), *t))
            .collect();
        let said = report("o/r", "2026-11-04", 7, 5, 1, &jobs, &took);
        assert!(
            said.starts_with(
                "ci list o/r, 2026-11-04 to 2026-11-10 (UTC, 7 days): 5 runs, 5 jobs ran\n"
            ),
            "{said}"
        );
        assert!(said.contains("1 runs unread"), "{said}");
        let row = |s: &str| {
            said.lines()
                .find(|l| l.trim_start().starts_with(s))
                .unwrap_or_default()
                .to_string()
        };
        assert!(
            row("mac ").ends_with("3 jobs, wait median 30 s, p90 600 s"),
            "{said}"
        );
        assert!(
            row("vps ").ends_with("1 jobs, wait median 4 s, p90 4 s"),
            "{said}"
        );
        assert!(
            row("hosted ").ends_with("1 jobs, wait median 6 s, p90 6 s"),
            "{said}"
        );
        assert!(
            row("gate: the gate runner").ends_with("2 jobs, wait median 30 s, p90 30 s"),
            "{said}"
        );
        assert!(
            row("route: the router's Mac pool").ends_with("1 jobs, wait median 600 s, p90 600 s"),
            "{said}"
        );
        assert!(
            row("explainer-gate.yml gate (vps)").ends_with("1 jobs, wait median 4 s, p90 4 s"),
            "{said}"
        );
        // done-when 2: a gate's time, before and after, read from green runs
        assert!(
            said.lines().last().unwrap_or_default().ends_with("4 runs, median 63 s, p90 227 s")
                && said.contains("green runs by workflow, attempt start to done (waits included):\n  explainer-gate.yml (mac) "),
            "{said}"
        );

        let vps = report(
            "o/r",
            "2026-11-04",
            7,
            1,
            0,
            &[job("site.yml", "test", &["site-perso-vps"], 9)],
            &[],
        );
        assert!(
            vps.contains("on the Mac, why:\n  none: no job ran on the Mac\n")
                && vps.ends_with("(waits included):\n  none: no green run\n"),
            "{vps}"
        );
        assert!(
            vps.contains("  mac ") && vps.contains(" 0 jobs, wait median 0 s"),
            "{vps}"
        );
    }

    /// A day's runs are listed per day, a finished run's jobs read once and kept: the second list asks for no job, an
    /// unfinished run is never cached, and the budget's floor stops the reads.
    #[test]
    fn a_finished_run_is_read_once_then_served_from_the_cache() {
        let _budget = TestBudget::new();
        let dir = std::env::temp_dir().join(format!("ci-list-{}", std::process::id()));
        let cache = dir.join("jobs.jsonl");
        let _ = std::fs::remove_dir_all(&dir);
        let asked = Mutex::new(vec![]);
        let remaining = Mutex::new(FLOOR + 100);
        let attempt = Mutex::new(1);
        let ask = |args: &[&str]| -> Result<String, String> {
            asked.lock().expect("lock").push(args.join(" "));
            let path = args
                .iter()
                .copied()
                .find(|a| a.starts_with("repos/") || *a == "rate_limit")
                .unwrap_or_default();
            Ok(match path {
                p if p.contains("created=2026-11-04") => {
                    let attempt = *attempt.lock().expect("lock");
                    format!("1\t.github/workflows/site.yml\tcompleted\t{attempt}\tsuccess\t2026-11-04T10:00:00Z\t2026-11-04T10:01:03Z\n2\t.github/workflows/explainer-gate.yml\tin_progress\t1\t\t2026-11-04T10:00:00Z\t2026-11-04T10:00:09Z\n")
                }
                p if p.contains("created=") => String::new(),
                "rate_limit" => remaining.lock().expect("lock").to_string(),
                p if p.ends_with("runs/1/jobs?filter=all&per_page=100") && args.contains(&"--slurp") => json!([{"jobs": [
                    {"name": "test", "labels": ["site-perso-vps"], "created_at": "2026-11-04T10:00:00Z",
                     "started_at": "2026-11-04T10:00:42Z", "conclusion": "success"},
                    {"name": "define-ci", "labels": ["self-hosted", "site-perso-gate"], "created_at": "2026-11-04T10:00:00Z",
                     "started_at": "2026-11-04T10:00:00Z", "conclusion": "skipped"},
                    {"name": "route", "labels": ["self-hosted", "site-perso-gate"], "created_at": "2026-11-04T10:00:00Z",
                     "started_at": "2026-11-04T10:00:01Z", "conclusion": "success"}]}])
                .to_string(),
                p => return Err(format!("unexpected {p}")),
            })
        };
        let said = list(&ask, "o/r", "2026-11-04", 2, &cache).expect("list");
        assert!(
            said.contains(": 2 runs, 2 jobs ran\n  1 runs unread"),
            "{said}"
        );
        assert!(
            said.contains("site.yml test (vps)") && said.contains("wait median 42 s"),
            "{said}"
        );
        assert!(
            said.contains("  site.yml (mac+vps) ")
                && said.contains("1 runs, median 63 s, p90 63 s\n")
                && !said.contains("explainer-gate.yml"),
            "only a green run is timed, placed by every job that ran: {said}"
        );
        let days: Vec<String> = asked
            .lock()
            .expect("lock")
            .iter()
            .filter(|a| a.contains("created="))
            .cloned()
            .collect();
        assert_eq!(days.len(), 2, "one listing a day: {days:?}");
        assert!(days[1].contains("created=2026-11-05"), "{days:?}");

        asked.lock().expect("lock").clear();
        let again = list(&ask, "o/r", "2026-11-04", 2, &cache).expect("list");
        assert_eq!(said, again);
        assert!(
            !asked
                .lock()
                .expect("lock")
                .iter()
                .any(|a| a.contains("/jobs")),
            "read from the cache"
        );

        // a re-run after the cache: its run is read again, every attempt's jobs, and the newer line wins
        *attempt.lock().expect("lock") = 2;
        let rerun = list(&ask, "o/r", "2026-11-04", 2, &cache).expect("list");
        let reads = |asked: &Mutex<Vec<String>>| {
            asked
                .lock()
                .expect("lock")
                .iter()
                .filter(|a| a.contains("/jobs"))
                .count()
        };
        assert_eq!(reads(&asked), 1, "read again");
        // its jobs are every attempt's, its time attempt 2's alone: a re-run is not timed, so the two never mix
        let jobs_part = |s: &str| s.split("green runs").next().unwrap_or_default().to_string();
        assert_eq!(jobs_part(&rerun), jobs_part(&said));
        assert!(
            rerun.ends_with("(waits included):\n  none: no green run\n"),
            "{rerun}"
        );
        asked.lock().expect("lock").clear();
        list(&ask, "o/r", "2026-11-04", 2, &cache).expect("list");
        assert_eq!(reads(&asked), 0, "attempt 2 is cached");
        *attempt.lock().expect("lock") = 1;

        // a cache it cannot write fails before any call: the 2026-11-11 first read spent 2,600 calls, then was refused
        asked.lock().expect("lock").clear();
        let refused = list(
            &ask,
            "o/r",
            "2026-11-04",
            2,
            &cache.join("under-a-file.jsonl"),
        );
        assert!(refused.is_err(), "{refused:?}");
        assert!(
            asked.lock().expect("lock").is_empty(),
            "no call before the cache opens"
        );

        // past the floor nothing is read: the new run stays unread, and says so
        let _ = std::fs::remove_file(&cache);
        *remaining.lock().expect("lock") = FLOOR;
        let held = list(&ask, "o/r", "2026-11-04", 2, &cache).expect("list");
        assert!(
            held.contains(": 2 runs, 0 jobs ran\n  2 runs unread"),
            "{held}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn github_times_read_to_unix_seconds() {
        let _budget = TestBudget::new();
        assert_eq!(unix("1970-01-02T00:01:05Z"), Some(86_465));
        assert_eq!(
            unix("2026-11-08T15:22:34Z").map(|s| s % 86_400),
            Some(15 * 3600 + 22 * 60 + 34)
        );
        assert_eq!(unix("2026-11-08"), None);
    }
}
