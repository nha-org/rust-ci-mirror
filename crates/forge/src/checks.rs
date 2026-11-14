//! A head's checks as a merge reads them, and the files its base changed since: merge-claim.py's `--checks` and `--stale` in
//! Rust (task rust-hooks-rest-port; @nha, 2026-11-10: « all these python scripts are also very brittle. we want to move all
//! logic to rust and have it well tested. »). `dash ci wait`, the merge on green and `dash ci checks|stale` read them here; the
//! .py stays for the older app's Orch.Rerun alone (the task's dialog: « Keep the two .py for Elixir »), so a rule changed here
//! is changed there too. Each rule's incident is in `git log -- .claude/hooks/merge-claim.py`.
use crate::mirror::CONTEXT as MIRROR;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

/// What dash ci wait and the merge on green count as passed.
pub const GREEN: [&str; 3] = ["SUCCESS", "NEUTRAL", "SKIPPED"];

/// A head's checks, `[name, state]`, as `checks::checks` reads them and `dash ci checks` prints them.
pub type Rows = Vec<(String, String)>;

/// `gh` with these arguments: its stdout, or `None` when it fails or does not answer. Sync: the annotation reads run at once.
pub type Gh<'a> = &'a (dyn Fn(&[&str]) -> Option<String> + Sync);

/// One gh call's bound, the hook's own.
const CALL: Duration = Duration::from_secs(10);
/// The status contexts a check job that ran stands in for (Dashboard.CiRouter.taken/0).
const TAKEN: [&str; 2] = ["buildkite/dashboard", "ci/circleci: check"];
/// A job GitHub never started over billing says so in its annotation (Dashboard.Forge.GitHub's read_verdict): change both.
const BILLING: [&str; 2] = ["spending limit", "account payments have failed"];

/// The real gh in `cwd`, past gh-meter's low-quota delay (GH_URGENT): a merge's read waits on it.
pub fn gh_in<'a>(bin: &'a str, cwd: &'a Path) -> impl Fn(&[&str]) -> Option<String> + Sync + 'a {
    move |args: &[&str]| {
        let (status, out, _) =
            dash_process::answer(bin, args, Some(cwd), &[("GH_URGENT", "1")], CALL).ok()?;
        status.success().then_some(out)
    }
}

fn api(gh: Gh, path: &str) -> Option<Value> {
    serde_json::from_str(&gh(&["api", path])?).ok()
}

/// A check run's state: unfinished is PENDING, else its conclusion; a status context its state; a required context GitHub still
/// awaits (EXPECTED) is PENDING, never red.
fn state(x: &Value) -> String {
    let unfinished = x.get("status").is_some_and(|s| s != "COMPLETED");
    if unfinished || x["state"] == "EXPECTED" {
        return "PENDING".into();
    }
    let said = |k: &str| x[k].as_str().filter(|s| !s.is_empty());
    said("conclusion")
        .or(said("state"))
        .unwrap_or("?")
        .to_uppercase()
}

fn word(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '.' | '-')
}

/// A job's link, `/<owner>/<repo>/actions/runs/<run>/job/<job>`: (owner/repo, run, job).
fn run_of(x: &Value) -> Option<(String, u64, u64)> {
    let url = x["detailsUrl"].as_str()?;
    let at = url.find("/actions/runs/")?;
    let mut parts = url[..at].rsplit('/');
    let (repo, owner) = (parts.next()?, parts.next()?);
    let ok = |s: &str| !s.is_empty() && s.chars().all(word);
    let (run, rest) = url[at + 14..].split_once("/job/")?;
    let job: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let (run, job) = (run.parse().ok()?, job.parse().ok()?);
    (ok(owner) && ok(repo)).then(|| (format!("{owner}/{repo}"), run, job))
}

/// Each job (workflow and name) keeps, in each run, its newest attempt (the highest job id); across runs only a cancel or a skip
/// gives way to a newer run of the job: a red push run beside a green pull_request run stays. A row with no run link stays.
/// A job's key in one run: (workflow, name, run).
type Job = (Value, Value, u64);

fn latest(rows: Vec<Value>) -> Vec<Value> {
    let mut rest = vec![];
    // key -> (job id, row), in the order first seen
    let mut top: Vec<(Job, (u64, Value))> = vec![];
    for x in rows {
        let Some((_, run, job)) = run_of(&x) else {
            rest.push(x);
            continue;
        };
        let k = (x["workflowName"].clone(), x["name"].clone(), run);
        match top.iter_mut().find(|(t, _)| *t == k) {
            Some((_, kept)) if job > kept.0 => *kept = (job, x),
            Some(_) => {}
            None => top.push((k, (job, x))),
        }
    }
    let newest = |wf: &Value, name: &Value| {
        let same = top.iter().filter(|((w, n, _), _)| w == wf && n == name);
        same.map(|((_, _, r), _)| *r).max().unwrap_or_default()
    };
    let kept: Vec<Value> = (top.iter())
        .filter(|((w, n, r), (_, x))| {
            *r == newest(w, n) || !matches!(state(x).as_str(), "CANCELLED" | "SKIPPED")
        })
        .map(|(_, (_, x))| x.clone())
        .collect();
    rest.into_iter().chain(kept).collect()
}

/// The statuses the router withdrew: Buildkite builds (`bk-<n>`) and CircleCI links, the older app's live files.
pub type Withdrawn = (HashSet<String>, HashSet<String>);

/// Dashboard.Store.live_dir/0's ci-buildkite-withdrawn.json and ci-circleci-withdrawn.json; none under MERGE_CLAIM_SELFTEST.
fn withdrawn_files() -> Withdrawn {
    if std::env::var_os("MERGE_CLAIM_SELFTEST").is_some() {
        return Default::default();
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let live =
        std::env::var("CRM_LIVE_DIR").unwrap_or(format!("{home}/repo/site_perso/private/live"));
    let read = |f: &str| -> HashSet<String> {
        let text = std::fs::read_to_string(Path::new(&live).join(f)).unwrap_or_default();
        let list: Vec<Value> = serde_json::from_str(&text).unwrap_or_default();
        list.iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect()
    };
    (
        read("ci-buildkite-withdrawn.json"),
        read("ci-circleci-withdrawn.json"),
    )
}

/// A pending status the router withdrew (Dashboard.Ci.Buildkite.withdrawn?/1): its failure stays.
fn withdrawn(x: &Value, (bk, cc): &Withdrawn) -> bool {
    let url = x["targetUrl"].as_str().unwrap_or_default();
    let st = x["state"].as_str().unwrap_or_default();
    match x["context"].as_str() {
        Some("buildkite/dashboard") => {
            let build = url.split_once("/builds/").map(|(_, b)| {
                b.chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
            });
            matches!(st, "PENDING" | "EXPECTED")
                && build.is_some_and(|b| !b.is_empty() && bk.contains(&format!("bk-{b}")))
        }
        Some("ci/circleci: check") => {
            matches!(st, "PENDING" | "EXPECTED" | "ERROR")
                && cc.contains(url.split('?').next().unwrap_or_default())
        }
        _ => false,
    }
}

/// A Buildkite or CircleCI pending status gives way once dashboard.yml's check job ran on the head (any state but skipped or
/// cancelled); a withdrawn one reads pending until a job took its place. Dashboard.Ci.Buildkite.superseded/1: change both.
fn superseded(rows: Vec<Value>, files: &Withdrawn) -> Vec<Value> {
    let check = |n: &str| {
        n == "check"
            || n.strip_prefix("check (")
                .and_then(|r| r.strip_suffix(')'))
                .is_some_and(|d| !d.is_empty() && d.chars().all(|c| c.is_ascii_digit()))
    };
    let ran = rows.iter().any(|x| {
        x["workflowName"] == "dashboard"
            && check(x["name"].as_str().unwrap_or_default())
            && !matches!(x["conclusion"].as_str(), Some("SKIPPED" | "CANCELLED"))
    });
    let (gone, mut rows): (Vec<Value>, Vec<Value>) =
        rows.into_iter().partition(|x| withdrawn(x, files));
    let taken = |x: &Value| TAKEN.iter().any(|t| x["context"] == *t);
    if !(ran || rows.iter().any(taken)) {
        rows.extend(gone.into_iter().map(|mut x| {
            x["state"] = json!("PENDING");
            x
        }));
    }
    rows.retain(|x| {
        !(ran && taken(x) && matches!(x["state"].as_str(), Some("PENDING" | "EXPECTED")))
    });
    rows
}

/// rust/ci-mirror.txt's lines under `root`; none read, none.
fn mirror_list(root: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(root.join("rust/ci-mirror.txt")).unwrap_or_default();
    let lines = text.lines().map(str::trim);
    lines
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect()
}

/// The mirror's verdict says something of this file: the list names it, or it is the list. A list not read counts all of rust/,
/// a wait being the safe side. Dashboard.CiPaths's mirrored?/1: change both.
fn mirrored(path: &str, list: &[String]) -> bool {
    if list.is_empty() {
        return path.starts_with("rust/");
    }
    let named = |x: &String| match x.ends_with('/') {
        true => path.starts_with(&format!("rust/{x}")),
        false => path == format!("rust/{x}"),
    };
    path == "rust/ci-mirror.txt" || list.iter().any(named)
}

/// A PR's rollup, its mirror status EXPECTED when a site_perso PR touching what the mirror tests has none yet (gh lists a PR's
/// first 100 files: a bigger one counts as touching it). Dashboard.CiMirror's awaited?/1: change both.
fn mirror_rows(p: &Value, list: &[String]) -> Vec<Value> {
    let mut rows = p["statusCheckRollup"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let files = p["files"].as_array().cloned().unwrap_or_default();
    let more = p["changedFiles"].as_u64().unwrap_or_default() > files.len() as u64;
    let touches = more
        || files
            .iter()
            .any(|f| mirrored(f["path"].as_str().unwrap_or_default(), list));
    let url = p["url"].as_str().unwrap_or_default();
    let ours = url
        .rsplit_once("/site_perso/pull/")
        .is_some_and(|(_, n)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
    if ours && touches && !rows.iter().any(|x| x["context"] == MIRROR) {
        rows.push(json!({"context": MIRROR, "state": "EXPECTED"}));
    }
    rows
}

/// Whether one of the job's annotations says `says`: `None` when they did not answer.
fn annotated(gh: Gh, repo: &str, job: u64, says: impl Fn(&str) -> bool) -> Option<bool> {
    let notes = api(
        gh,
        &format!("repos/{repo}/check-runs/{job}/annotations?per_page=100"),
    )?;
    let notes = notes.as_array()?;
    Some(
        notes
            .iter()
            .any(|n| says(&n["message"].as_str().unwrap_or_default().to_lowercase())),
    )
}

fn billing(text: &str) -> bool {
    let text = text.to_lowercase();
    BILLING.iter().any(|b| text.contains(b))
}

/// A failed job GitHub never started over billing: no step, its annotation says why; any doubt is a real failure. A status
/// context in error quotes it only as the mirror's verdict (dash ci mirror's, of a job GitHub refused).
fn refused(x: &Value, gh: Gh) -> bool {
    let Some((repo, _, job)) = run_of(x) else {
        return x["context"] == MIRROR
            && state(x) == "ERROR"
            && billing(x["description"].as_str().unwrap_or_default());
    };
    if !matches!(
        state(x).as_str(),
        "FAILURE" | "STARTUP_FAILURE" | "CANCELLED"
    ) {
        return false;
    }
    let steps = api(gh, &format!("repos/{repo}/actions/jobs/{job}"));
    let no_step =
        steps.is_some_and(|j| j.is_object() && j["steps"].as_array().is_none_or(Vec::is_empty));
    no_step && annotated(gh, &repo, job, billing) == Some(true)
}

/// A check skipped for want of its tool says so in its annotation, these exact words (site.yml's old oci step, PR 973's image):
/// never green. Exact, so a lint's « type check skipped » stays green. `None` when the annotations did not answer.
fn not_run(x: &Value, gh: Gh) -> Option<bool> {
    let Some((repo, _, job)) = run_of(x) else {
        return Some(false);
    };
    let says = |m: &str| {
        m.contains("check not run: tool missing")
            || m.trim_end_matches('\n').ends_with("; image check skipped")
    };
    annotated(gh, &repo, job, says)
}

/// `gh pr view` of `pr` (a number, a link, a branch) in `repo`, its JSON `fields`.
fn pr_view(gh: Gh, pr: &str, repo: Option<&str>, fields: &str) -> Option<Value> {
    let mut args = vec!["pr", "view", pr];
    if let Some(repo) = repo.filter(|_| !pr.starts_with("http")) {
        args.extend(["-R", repo]);
    }
    args.extend(["--json", fields]);
    serde_json::from_str(&gh(&args)?).ok()
}

/// owner/repo of the checkout at `cwd`, from its origin's GitHub URL.
fn origin(cwd: &Path) -> Option<String> {
    let url = dash_process::run(
        "git",
        &["remote", "get-url", "origin"],
        Some(cwd),
        Duration::from_secs(5),
    )
    .ok()?;
    let (_, rest) = url.trim().split_once("github.com")?;
    let rest = rest.strip_prefix([':', '/'])?;
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let (owner, repo) = rest.split_once('/')?;
    let ok = |s: &str| !s.is_empty() && s.chars().all(word);
    (ok(owner) && ok(repo)).then(|| rest.to_string())
}

/// A commit sha, never a PR number: 7 to 40 hex, not all digits under 40.
fn is_sha(t: &str) -> bool {
    let hex = (7..=40).contains(&t.len())
        && t.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase());
    hex && !(t.len() < 40 && t.chars().all(|c| c.is_ascii_digit()))
}

/// The rows a head reads before its rules: a PR's rollup, or a commit's check runs and mirror status.
fn read(
    gh: Gh,
    target: &str,
    repo: Option<&str>,
    cwd: &Path,
    list: &[String],
) -> Option<Vec<Value>> {
    if !is_sha(target) {
        return Some(mirror_rows(
            &pr_view(gh, target, repo, "url,statusCheckRollup,files,changedFiles")?,
            list,
        ));
    }
    let repo = repo
        .map(String::from)
        .or_else(|| origin(cwd))
        .unwrap_or("{owner}/{repo}".into());
    let runs = format!("repos/{repo}/commits/{target}/check-runs?per_page=100");
    let runs_q = "[.check_runs[] | {name, status: (.status | ascii_upcase), conclusion: (.conclusion // \"\" | ascii_upcase), detailsUrl: .details_url}]";
    let status = format!("repos/{repo}/commits/{target}/status");
    let status_q = format!(
        "[.statuses[] | select(.context == \"{MIRROR}\") | {{context, state: (.state | ascii_upcase), description, detailsUrl: .target_url}}]"
    );
    let mut rows = vec![];
    for (path, q) in [(&runs, runs_q), (&status, status_q.as_str())] {
        let got: Value = serde_json::from_str(&gh(&["api", path, "-q", q])?).ok()?;
        rows.extend(got.as_array().cloned().unwrap_or_default());
    }
    // a head sha is a PR's as well: no mirror status yet on an open Rust PR is pending there too, kept when it heads them
    if !rows.iter().any(|x| x["context"] == MIRROR) {
        let pulls = api(gh, &format!("repos/{repo}/commits/{target}/pulls"))?;
        let heads = pulls.as_array().cloned().unwrap_or_default();
        let heads = heads.iter().filter(|p| {
            p["state"] == "open"
                && p["head"]["sha"]
                    .as_str()
                    .unwrap_or_default()
                    .starts_with(target)
        });
        for p in heads {
            let n = p["number"].to_string();
            let own = Some(repo.as_str()).filter(|r| !r.contains('{'));
            let view = pr_view(gh, &n, own, "url,files,changedFiles")?;
            if !mirror_rows(&view, list).is_empty() {
                rows.push(json!({"context": MIRROR, "state": "EXPECTED"}));
                break;
            }
        }
    }
    Some(rows)
}

/// The checks of `target`'s head (a PR's number, link or branch, or a commit sha), `[name, state]`: each job's latest attempt, a
/// superseded cancel dropped, a job GitHub refused for billing BILLING, a green job whose annotation says its check did not run
/// NOT_RUN, one whose annotations did not answer PENDING. `None` when gh does not answer. `root` holds rust/ci-mirror.txt.
pub fn checks(gh: Gh, target: &str, repo: Option<&str>, cwd: &Path, root: &Path) -> Option<Rows> {
    checks_with(gh, target, repo, cwd, root, &withdrawn_files())
}

fn checks_with(
    gh: Gh,
    target: &str,
    repo: Option<&str>,
    cwd: &Path,
    root: &Path,
    files: &Withdrawn,
) -> Option<Rows> {
    let kept = latest(superseded(
        read(gh, target, repo, cwd, &mirror_list(root))?,
        files,
    ));
    // once one red job is a real failure, no more gh calls: the merge is refused anyway
    let mut real = false;
    let mut out: Rows = vec![];
    for x in &kept {
        let mut st = state(x);
        if !GREEN.contains(&st.as_str()) && !real {
            if refused(x, gh) {
                st = "BILLING".into();
            } else {
                real = st != "PENDING";
            }
        }
        let name = ["name", "context"]
            .iter()
            .find_map(|k| x[*k].as_str().filter(|s| !s.is_empty()));
        out.push((name.unwrap_or("?").into(), st));
    }
    // an otherwise green head: each SUCCESS or NEUTRAL job's annotations read at once (in turn they outran the hook's bound)
    if out
        .iter()
        .all(|(_, s)| GREEN.contains(&s.as_str()) || s == "BILLING")
    {
        let said: Vec<Option<bool>> = std::thread::scope(|s| {
            let reads: Vec<_> = (kept.iter().zip(&out))
                .map(|(x, (_, st))| {
                    let read = matches!(st.as_str(), "SUCCESS" | "NEUTRAL");
                    s.spawn(move || if read { not_run(x, gh) } else { Some(false) })
                })
                .collect();
            reads
                .into_iter()
                .map(|h| h.join().unwrap_or(None))
                .collect()
        });
        for ((_, st), said) in out.iter_mut().zip(said) {
            match said {
                Some(true) => *st = "NOT_RUN".into(),
                None => *st = "PENDING".into(),
                Some(false) => {}
            }
        }
    }
    Some(out)
}

/// `(head, files)`: the files `pr` changes that its base changed since its head last met it, never tested together (#661 merged
/// on a run older than #635's change to four of its files; task merge-fresh-base). `None` when gh does not answer. The compare
/// lists 300 files at most: past that, all the PR's. ponytail: gh lists a PR's first 100 files; a bigger PR is checked on those.
pub fn stale(gh: Gh, pr: &str, repo: Option<&str>) -> Option<(String, Vec<String>)> {
    let p = pr_view(gh, pr, repo, "url,headRefOid,baseRefName,files")?;
    let url = p["url"].as_str()?;
    let (_, rest) = url.split_once("github.com/")?;
    let (owner_repo, _) = rest.split_once("/pull/")?;
    let head = p["headRefOid"].as_str().filter(|h| !h.is_empty())?;
    let base = p["baseRefName"]
        .as_str()
        .filter(|b| !b.is_empty())
        .unwrap_or("main");
    let diff = api(gh, &format!("repos/{owner_repo}/compare/{head}...{base}"))?;
    // no `files` in the compare is nothing moved, as the .py's `.get("files") or []` read it (/code-review)
    let moved: HashSet<&str> = diff["files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| f["filename"].as_str())
        .collect();
    let mine = p["files"].as_array().cloned().unwrap_or_default();
    let mut mine: Vec<String> = mine
        .iter()
        .filter_map(|f| f["path"].as_str().map(String::from))
        .collect();
    mine.sort();
    mine.dedup();
    if moved.len() < 300 {
        mine.retain(|f| moved.contains(f.as_str()));
    }
    Some((head.into(), mine))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;
    use std::sync::Mutex;

    fn ok() -> Value {
        json!({"name": "check", "status": "COMPLETED", "conclusion": "SUCCESS"})
    }

    fn run(n: u64, c: &str) -> Value {
        json!({"name": "check", "status": "COMPLETED", "conclusion": c, "workflowName": "dash",
               "detailsUrl": format!("https://github.com/o/r/actions/runs/{n}/job/{n}0")})
    }

    fn with(x: &Value, k: &str, v: Value) -> Value {
        let mut x = x.clone();
        x[k] = v;
        x
    }

    /// A stand-in gh: `pr view` answers `view` (its rollup `rows`), `api` the first rule whose key the path holds.
    fn gh_of(
        rows: &[Value],
        api: &[(&str, Value)],
    ) -> impl Fn(&[&str]) -> Option<String> + Sync + use<> {
        let view = json!({"url": "https://github.com/o/r/pull/316", "statusCheckRollup": rows, "files": [], "changedFiles": 0});
        let api: Vec<(String, Value)> = api
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        move |args: &[&str]| match args {
            ["pr", "view", ..] => Some(view.to_string()),
            ["api", path, ..] => api
                .iter()
                .find(|(k, _)| path.contains(k.as_str()))
                .map(|(_, v)| v.to_string()),
            _ => None,
        }
    }

    /// The checks of PR 316 through `gh_of`, any annotation not named answering none.
    fn read(rows: &[Value], api: &[(&str, Value)]) -> Rows {
        let mut api = api.to_vec();
        api.push(("/annotations", json!([])));
        let gh = gh_of(rows, &api);
        let none = Default::default();
        checks_with(
            &gh,
            "316",
            Some("o/r"),
            Path::new("."),
            Path::new("/nowhere"),
            &none,
        )
        .expect("an answer")
    }

    fn rows(r: &[(&str, &str)]) -> Rows {
        r.iter()
            .map(|(n, s)| (n.to_string(), s.to_string()))
            .collect()
    }

    #[dash_test]
    fn checks_keep_each_jobs_latest_attempt_and_drop_a_superseded_cancel() {
        assert_eq!(
            read(&[run(1, "CANCELLED"), run(2, "SUCCESS")], &[]),
            rows(&[("check", "SUCCESS")])
        );
        assert_eq!(
            read(&[run(2, "SUCCESS"), run(1, "CANCELLED")], &[]),
            rows(&[("check", "SUCCESS")])
        );
        assert_eq!(
            read(&[run(1, "CANCELLED")], &[]),
            rows(&[("check", "CANCELLED")])
        );
        // an older run's green is no cancel: it stays beside the newer red
        assert_eq!(
            read(&[run(1, "SUCCESS"), run(2, "FAILURE")], &[]),
            rows(&[("check", "SUCCESS"), ("check", "FAILURE")])
        );
        // a red push run beside a newer green pull_request run of the same name stays
        let both = read(&[run(1, "FAILURE"), run(2, "SUCCESS")], &[]);
        assert!(
            both.contains(&("check".into(), "FAILURE".into())),
            "{both:?}"
        );
        // another workflow's newer green does not lift a cancel
        let site = with(&run(2, "SUCCESS"), "workflowName", json!("site"));
        assert!(
            read(&[run(1, "CANCELLED"), site], &[]).contains(&("check".into(), "CANCELLED".into()))
        );
        // a re-run attempt keeps its run id: the higher job id wins, a queued one too
        let att = |c: &str, j: u64| {
            with(
                &run(5, c),
                "detailsUrl",
                json!(format!("https://github.com/o/r/actions/runs/5/job/{j}")),
            )
        };
        assert_eq!(
            read(&[att("CANCELLED", 1), att("SUCCESS", 2)], &[]),
            rows(&[("check", "SUCCESS")])
        );
        let queued = with(&att("", 2), "status", json!("QUEUED"));
        assert_eq!(
            read(&[att("SUCCESS", 1), queued], &[]),
            rows(&[("check", "PENDING")])
        );
        // a status context; a required one GitHub awaits is pending, never red
        assert_eq!(
            read(&[json!({"context": "ci", "state": "EXPECTED"})], &[]),
            rows(&[("ci", "PENDING")])
        );
        assert_eq!(
            read(&[ok(), json!({"context": "ci", "state": "FAILURE"})], &[]),
            rows(&[("check", "SUCCESS"), ("ci", "FAILURE")])
        );
        assert_eq!(read(&[], &[]), rows(&[]), "no CI at all");
    }

    #[dash_test]
    fn checks_read_billing_only_for_a_job_with_no_step_and_its_annotation() {
        let note = json!([{"message": "The job was not started because recent account payments have failed or your spending limit needs to be increased."}]);
        let bill = run(3, "FAILURE");
        let gh = |job: Value, notes: Value| gh_of(&[], &[("/jobs/", job), ("/annotations", notes)]);
        assert!(refused(&bill, &gh(json!({"steps": []}), note.clone())));
        assert!(!refused(
            &bill,
            &gh(json!({"steps": [{"name": "x"}]}), note.clone())
        ));
        assert!(!refused(
            &bill,
            &gh(
                json!({"steps": []}),
                json!([{"message": "Process completed with exit code 1."}])
            )
        ));
        assert!(
            !refused(&bill, &gh_of(&[], &[])),
            "no answer is a real failure"
        );
        assert!(!refused(
            &run(3, "SUCCESS"),
            &gh(json!({"steps": []}), note.clone())
        ));
        assert!(refused(
            &run(3, "STARTUP_FAILURE"),
            &gh(json!({"steps": []}), note.clone())
        ));
        let words = "not run: GitHub refused the mirror's job (spending limit or account payments have failed)";
        let none = gh_of(&[], &[]);
        assert!(refused(
            &json!({"context": MIRROR, "state": "ERROR", "description": words}),
            &none
        ));
        assert!(!refused(
            &json!({"context": "ci", "state": "ERROR", "description": words}),
            &none
        ));
        assert!(!refused(
            &json!({"context": MIRROR, "state": "ERROR", "description": "other"}),
            &none
        ));
        // through the read: BILLING, and beside a real failure the real one is said
        let api = [("/jobs/", json!({"steps": []})), ("/annotations", note)];
        assert_eq!(
            read(std::slice::from_ref(&bill), &api),
            rows(&[("check", "BILLING")])
        );
        let other = with(&ok(), "conclusion", json!("FAILURE"));
        let other = with(&other, "name", json!("other"));
        // a row with no run link reads first: its real failure stops the billing read, the merge is refused anyway
        assert_eq!(
            read(&[bill, other], &api),
            rows(&[("other", "FAILURE"), ("check", "FAILURE")])
        );
    }

    #[dash_test]
    fn checks_read_a_check_skipped_for_want_of_its_tool_as_not_run() {
        let skip = json!([{"message": "container's services do not answer this runner (bin/ci-runner starts them; sandbox/runner.sb lets the job reach them); image check skipped"}]);
        let img = json!({"name": "test", "workflowName": "site", "status": "COMPLETED", "conclusion": "SUCCESS",
                         "detailsUrl": "https://github.com/nha-org/site_perso/actions/runs/41539049599/job/125615729047"});
        let says = |m: Value| not_run(&img, &gh_of(&[], &[("/annotations", m)]));
        assert_eq!(says(skip.clone()), Some(true));
        assert_eq!(
            says(json!([{"message": "c; check not run: tool missing"}])),
            Some(true)
        );
        assert_eq!(
            says(json!([{"message": "Node 20 is deprecated"}])),
            Some(false)
        );
        assert_eq!(
            says(json!([{"message": "type check skipped for vendored files"}])),
            Some(false)
        );
        assert_eq!(not_run(&img, &gh_of(&[], &[])), None, "unread");
        assert_eq!(
            not_run(&ok(), &gh_of(&[], &[("/annotations", skip.clone())])),
            Some(false),
            "no run link"
        );
        let define = with(&run(6, "SUCCESS"), "name", json!("define-ci"));
        let api = [
            ("/125615729047/annotations", skip),
            ("/annotations", json!([])),
        ];
        assert_eq!(
            read(&[img.clone(), define.clone()], &api),
            rows(&[("test", "NOT_RUN"), ("define-ci", "SUCCESS")])
        );
        assert_eq!(
            read(&[with(&img, "conclusion", json!("NEUTRAL"))], &api),
            rows(&[("test", "NOT_RUN")])
        );
        // its annotations unread: pending, never green
        let unread = gh_of(&[img], &[]);
        let got = checks_with(
            &unread,
            "316",
            None,
            Path::new("."),
            Path::new("/nowhere"),
            &Default::default(),
        );
        assert_eq!(got, Some(rows(&[("test", "PENDING")])));
    }

    #[dash_test]
    fn checks_let_a_buildkite_or_circleci_status_give_way_to_a_check_job_that_ran() {
        let bk = json!({"context": "buildkite/dashboard", "state": "PENDING"});
        let chk = |c: &str| json!({"workflowName": "dashboard", "name": "check (1)", "status": "COMPLETED", "conclusion": c});
        assert_eq!(
            read(&[bk.clone(), chk("SUCCESS")], &[]),
            rows(&[("check (1)", "SUCCESS")])
        );
        assert!(
            read(&[bk.clone(), chk("SKIPPED")], &[])
                .contains(&("buildkite/dashboard".into(), "PENDING".into()))
        );
        assert!(
            read(&[bk.clone(), chk("CANCELLED"), chk("SKIPPED")], &[])
                .contains(&("buildkite/dashboard".into(), "PENDING".into()))
        );
        let failed = with(&bk, "state", json!("FAILURE"));
        assert!(
            read(&[failed, chk("SUCCESS")], &[])
                .contains(&("buildkite/dashboard".into(), "FAILURE".into()))
        );
        let cc = json!({"context": "ci/circleci: check", "state": "PENDING"});
        assert_eq!(
            read(&[cc.clone(), chk("SUCCESS")], &[]),
            rows(&[("check (1)", "SUCCESS")])
        );
        // a withdrawn CircleCI error goes once the check ran, else reads pending; its failure stays
        let link = "https://circleci.com/gh/nha-org/site_perso/7";
        let files: Withdrawn = (HashSet::new(), HashSet::from([link.to_string()]));
        let err = json!({"context": "ci/circleci: check", "state": "ERROR", "targetUrl": format!("{link}?utm_source=x")});
        assert!(
            withdrawn(&err, &files) && !withdrawn(&with(&err, "state", json!("FAILURE")), &files)
        );
        assert!(!withdrawn(&err, &Default::default()));
        let bk12 = json!({"context": "buildkite/dashboard", "state": "PENDING", "targetUrl": "https://buildkite.com/x/builds/12"});
        assert!(withdrawn(
            &bk12,
            &(HashSet::from(["bk-12".to_string()]), HashSet::new())
        ));
        assert_eq!(
            superseded(vec![err.clone(), chk("SUCCESS")], &files),
            vec![chk("SUCCESS")]
        );
        assert_eq!(superseded(vec![err.clone()], &files)[0]["state"], "PENDING");
        let red = superseded(
            vec![with(&err, "state", json!("FAILURE")), chk("SUCCESS")],
            &files,
        );
        assert_eq!(red.len(), 2, "its failure stays: {red:?}");
    }

    #[dash_test]
    fn checks_owe_the_mirror_on_a_site_perso_pr_touching_rust() {
        let pr = |path: &str, more: u64| {
            json!({"url": "https://github.com/nha-org/site_perso/pull/1500", "changedFiles": more,
                                                 "files": [{"path": path}], "statusCheckRollup": [ok()]})
        };
        let list = vec!["crates/".to_string(), "Cargo.toml".to_string()];
        assert_eq!(
            mirror_rows(&pr("rust/crates/forge/src/lib.rs", 1), &list)
                .last()
                .expect("a row")["context"],
            MIRROR
        );
        assert_eq!(
            mirror_rows(&pr("private/docs/x.html", 1), &list).len(),
            1,
            "nothing the mirror tests"
        );
        assert_eq!(
            mirror_rows(&pr("private/docs/x.html", 101), &list).len(),
            2,
            "past gh's first files: counted as touching"
        );
        assert_eq!(
            mirror_rows(&pr("rust/ci-mirror.txt", 1), &list).len(),
            2,
            "the list itself"
        );
        assert_eq!(
            mirror_rows(&pr("rust/other.md", 1), &list).len(),
            1,
            "rust/ but off the list"
        );
        assert!(
            mirrored("rust/anything", &[]),
            "a list not read counts all of rust/"
        );
        let other = with(
            &pr("rust/crates/x.rs", 1),
            "url",
            json!("https://github.com/o/r/pull/9"),
        );
        assert_eq!(
            mirror_rows(&other, &list).len(),
            1,
            "another repo owes no mirror"
        );
    }

    #[dash_test]
    fn checks_read_a_commit_sha_through_its_check_runs() {
        let asked = Mutex::new(vec![]);
        let runs = json!([{"name": "check", "status": "COMPLETED", "conclusion": "CANCELLED", "detailsUrl": "https://github.com/o/r/actions/runs/1/job/10"},
                          {"name": "check", "status": "COMPLETED", "conclusion": "SUCCESS", "detailsUrl": "https://github.com/o/r/actions/runs/2/job/20"}]);
        let gh = |args: &[&str]| {
            asked.lock().expect("lock").push(args.join(" "));
            match args {
                ["api", p, ..] if p.contains("/check-runs?") => Some(runs.to_string()),
                ["api", p, ..]
                    if p.ends_with("/status")
                        || p.ends_with("/pulls")
                        || p.contains("/annotations") =>
                {
                    Some("[]".into())
                }
                _ => None,
            }
        };
        let got = checks_with(
            &gh,
            "0123abc",
            Some("o/r"),
            Path::new("."),
            Path::new("/nowhere"),
            &Default::default(),
        );
        assert_eq!(got, Some(rows(&[("check", "SUCCESS")])));
        assert!(
            asked.lock().expect("lock")[0]
                .starts_with("api repos/o/r/commits/0123abc/check-runs?per_page=100")
        );
        assert!(
            is_sha("0123abc") && !is_sha("1500") && !is_sha("0123ABC") && is_sha(&"1".repeat(40))
        );
        // gh silent: no answer, never green
        let silent = |_: &[&str]| None;
        assert_eq!(
            checks_with(
                &silent,
                "316",
                None,
                Path::new("."),
                Path::new("/nowhere"),
                &Default::default()
            ),
            None
        );
    }

    #[dash_test]
    fn stale_names_the_prs_files_its_base_changed_since_its_head() {
        let view = json!({"url": "https://github.com/o/r/pull/661", "headRefOid": "abc1234def", "baseRefName": "main",
                          "files": [{"path": "b.ex"}, {"path": "a.ex"}]});
        let asked = Mutex::new(vec![]);
        let gh_of = |changed: Option<Vec<String>>| {
            let view = view.clone();
            let asked = &asked;
            move |args: &[&str]| match args {
                ["pr", "view", ..] => Some(view.to_string()),
                ["api", path] => {
                    asked.lock().expect("lock").push(path.to_string());
                    let files: Vec<Value> = changed
                        .clone()?
                        .iter()
                        .map(|f| json!({"filename": f}))
                        .collect();
                    Some(json!({"files": files}).to_string())
                }
                _ => None,
            }
        };
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let got = stale(&gh_of(Some(names(&["b.ex", "c.ex"]))), "661", None);
        assert_eq!(got, Some(("abc1234def".into(), names(&["b.ex"]))));
        assert_eq!(
            asked.lock().expect("lock")[0],
            "repos/o/r/compare/abc1234def...main"
        );
        assert_eq!(
            stale(&gh_of(Some(names(&["c.ex"]))), "661", None)
                .expect("read")
                .1,
            Vec::<String>::new()
        );
        assert_eq!(
            stale(&gh_of(None), "661", None),
            None,
            "gh silent on the compare is no answer"
        );
        let many: Vec<String> = (0..300).map(|i| format!("x{i}")).collect();
        assert_eq!(
            stale(&gh_of(Some(many)), "661", None).expect("read").1,
            names(&["a.ex", "b.ex"]),
            "past the 300-file cap"
        );
    }
}
