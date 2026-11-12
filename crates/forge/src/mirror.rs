//! The Rust CI mirror's verdict as a merge reads it (task rust-pr-waits-mirror). Dashboard.CiMirror writes `ci-mirror/rust` on
//! a site_perso head and stops reading that head once it is red, so a rerun on the mirror never reached the PR: PR 1444's head
//! d321d07d read red through all three attempts of mirror run 43041329651 (2026-11-11). A red row is read again here from the
//! mirror's own check runs: one still running is no verdict yet, all green is green (written back on the head, so the PR shows
//! it too), and a settled red posts its failing step's last lines on the PR, once a job.
use serde_json::Value;

/// Dashboard.CiMirror's @context and merge-claim.py's MIRROR: change all three.
pub const CONTEXT: &str = "ci-mirror/rust";
/// The failing step's last lines a PR shows.
const TAIL: usize = 40;
/// A log line past this is cut: a comment holds 65536 characters.
const WIDE: usize = 300;
/// What a mirror check run concludes that holds nothing back (Dashboard.CiMirror.state/1).
pub const GREEN: [&str; 3] = ["success", "skipped", "neutral"];

/// `gh` with these arguments, in the site_perso checkout: its stdout.
pub type Gh<'a> = &'a mut dyn FnMut(&[&str]) -> Result<String, String>;

/// A red `ci-mirror/rust` read again.
#[derive(Debug, PartialEq)]
pub enum Reread {
    /// A run of its mirror commit is not done: a rerun, no verdict yet.
    Running,
    /// All its runs done and green, the head's status set to success.
    Green,
    /// Red still, its failing step's lines on the PR; `Some(why)` when they could not be posted.
    Red(Option<String>),
}

/// The red `ci-mirror/rust` of `target`, a PR (number or link) or a commit sha as the hook read it, read again from the mirror
/// commit its status names. A read that does not answer is an error, never the old red: the caller asks again. A red naming no
/// mirror commit (CiMirror's refusal: a secret scan hit, a broken list) or with no run is final, as the hook read it.
pub fn reread(target: &str, gh: Gh) -> Result<Reread, String> {
    let (pr, head) = pr_and_head(target, gh)?;
    let status = format!("repos/{{owner}}/{{repo}}/commits/{head}/status");
    let pick = format!(
        "[.statuses[] | select(.context == \"{CONTEXT}\")][0] | \"\\(.target_url) \\(.updated_at)\" // \"\""
    );
    let got = gh(&["api", &status, "--jq", &pick])?;
    let (url, red_at) = got.trim().split_once(' ').unwrap_or_default();
    let Some((mirror, commit)) = mirror_of(url) else {
        return Ok(Reread::Red(None));
    };
    // GitHub pages 30 by default: a failed run past them would read green (/code-review of 56ca68f41)
    let path = format!("repos/{mirror}/commits/{commit}/check-runs?per_page=100");
    let fields = "[.check_runs[] | {id, name, status, conclusion, html_url, started_at}]";
    let runs = gh(&["api", &path, "--jq", fields])?;
    let runs: Vec<Value> =
        serde_json::from_str(&runs).map_err(|e| format!("{CONTEXT}'s runs: {e}: {runs}"))?;
    // a rerun: a run not done that started after the red was written. One that started before it is the run CiMirror gave up
    // on after 24 hours, and that red stands (/code-review of 0906afff9). ISO times in UTC compare as text; none yet is new.
    let rerun = |r: &Value| {
        r["status"] != "completed" && r["started_at"].as_str().is_none_or(|at| at > red_at)
    };
    if runs.iter().any(rerun) {
        return Ok(Reread::Running);
    }
    if runs.is_empty() || runs.iter().any(|r| r["status"] != "completed") {
        return Ok(Reread::Red(None));
    }
    let green = |r: &&Value| GREEN.contains(&r["conclusion"].as_str().unwrap_or_default());
    let Some(failed) = runs.iter().find(|r| !green(r)) else {
        let said = format!(
            "description={}/{} check runs done, after a rerun on the mirror",
            runs.len(),
            runs.len()
        );
        let at_head = format!("repos/{{owner}}/{{repo}}/statuses/{head}");
        let target = format!("target_url={url}");
        let fields = [
            "-f",
            "state=success",
            "-f",
            &format!("context={CONTEXT}"),
            "-f",
            &said,
            "-f",
            &target,
        ];
        // green only once the head says so: a write that fails is no verdict, read again (/code-review of 0906afff9)
        gh(&[&["api", "-X", "POST", &at_head][..], &fields].concat())?;
        return Ok(Reread::Green);
    };
    let shown = pr.map_or(Ok(()), |pr| shown(&pr, &head, mirror, failed, gh));
    Ok(Reread::Red(shown.err()))
}

/// A target that is a commit sha, never a PR number: 7 to 40 hex digits, not all decimal. red::links reads it too.
pub(crate) fn sha(target: &str) -> bool {
    (7..=40).contains(&target.len())
        && target.bytes().all(|b| b.is_ascii_hexdigit())
        && !target.bytes().all(|b| b.is_ascii_digit())
}

/// The PR and its full head sha (a status is written on the full sha alone): a sha's open PR it heads, if any.
fn pr_and_head(target: &str, gh: Gh) -> Result<(Option<String>, String), String> {
    if !sha(target) {
        let got = gh(&[
            "pr",
            "view",
            target,
            "--json",
            "number,headRefOid",
            "-q",
            r#""\(.number) \(.headRefOid)""#,
        ])?;
        let (n, head) = got
            .trim()
            .split_once(' ')
            .ok_or(format!("gh pr view {target}: « {got} »"))?;
        return Ok((Some(n.to_string()), head.to_string()));
    }
    let commit = format!("repos/{{owner}}/{{repo}}/commits/{target}");
    let full = gh(&["api", &commit, "--jq", ".sha"])?;
    let full = full.trim();
    let pulls = format!("{commit}/pulls");
    let open =
        format!(r#"[.[] | select(.state == "open" and .head.sha == "{full}") | .number][0] // """#);
    let n = gh(&["api", &pulls, "--jq", &open])?;
    let n = n.trim();
    Ok(((!n.is_empty()).then(|| n.to_string()), full.to_string()))
}

/// `https://github.com/<owner>/<repo>/commit/<sha>/checks`: `(<owner>/<repo>, <sha>)`.
pub(crate) fn mirror_of(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("https://github.com/")?;
    let (repo, rest) = rest
        .match_indices('/')
        .nth(1)
        .map(|(i, _)| rest.split_at(i))?;
    let commit = rest.strip_prefix("/commit/")?.split('/').next()?;
    (!commit.is_empty()).then_some((repo, commit))
}

/// The failed job's last step lines, posted on the PR unless a comment names that job already (« ci-mirror/rust job <id> »).
fn shown(pr: &str, head: &str, mirror: &str, failed: &Value, gh: Gh) -> Result<(), String> {
    let id = failed["id"]
        .as_u64()
        .ok_or("a failed run with no id")?
        .to_string();
    let marker = format!("{CONTEXT} job {id}");
    let comments = format!("repos/{{owner}}/{{repo}}/issues/{pr}/comments");
    if gh(&["api", "--paginate", &comments, "--jq", ".[].body"])?.contains(&marker) {
        return Ok(());
    }
    let log = gh(&["run", "view", "--job", &id, "--log-failed", "-R", mirror])?;
    let body = comment(&marker, head, failed, &log);
    gh(&["pr", "comment", pr, "--body", &body]).map(drop)
}

/// The PR's comment: the failing step named, its last `TAIL` lines as `gh run view --log-failed` prints them (`job\tstep\t<time>
/// <line>`), the time and colour codes dropped.
fn comment(marker: &str, head: &str, failed: &Value, log: &str) -> String {
    let rows: Vec<(&str, String)> = log
        .lines()
        .filter_map(|l| {
            let mut fields = l.splitn(3, '\t');
            let (_, step, line) = (fields.next()?, fields.next()?, fields.next()?);
            let line = match line.split_once(' ') {
                Some((time, rest))
                    if time.ends_with('Z') && time.starts_with(|c: char| c.is_ascii_digit()) =>
                {
                    rest
                }
                _ => line,
            };
            Some((step, plain(line)))
        })
        .collect();
    let step = rows
        .last()
        .map_or("a step the log does not name", |(s, _)| s);
    let lines = rows[rows.len().saturating_sub(TAIL)..]
        .iter()
        .map(|(_, l)| l.as_str());
    let lines = lines.collect::<Vec<_>>().join("\n");
    let job = failed["name"].as_str().unwrap_or("?");
    let url = failed["html_url"].as_str().unwrap_or_default();
    let short = &head[..head.len().min(8)];
    format!(
        "**{marker} is red** on {short}: `{job}` failed at step « {step} ». Its last lines, as [the mirror's log]({url}) holds them:\n\n````text\n{lines}\n````\n"
    )
}

/// A log line without its terminal colour codes, cut at `WIDE` characters.
fn plain(line: &str) -> String {
    uncoloured(line).chars().take(WIDE).collect()
}

/// A log line without its terminal colour codes or a leading byte-order mark; red::causes reads it too.
pub(crate) fn uncoloured(line: &str) -> String {
    let mut out = String::new();
    let mut chars = line.trim_start_matches('\u{feff}').chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // ESC [ ... a final letter
            chars.by_ref().find(|c| c.is_ascii_alphabetic());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;

    pub(crate) const HEAD: &str = "d321d07d00000000000000000000000000000000";
    pub(crate) const COMMIT: &str = "3d312b9c222ec23cae423a2450993ea13b5560ef";

    /// A stand-in gh: each call's arguments kept, answered by the first rule whose words they all hold.
    pub(crate) fn stand_in<'a>(
        rules: &'a [(&'a [&'a str], &'a str)],
        calls: &'a mut Vec<Vec<String>>,
    ) -> impl FnMut(&[&str]) -> Result<String, String> + 'a {
        move |args: &[&str]| {
            calls.push(args.iter().map(|a| a.to_string()).collect());
            let joined = args.join(" ");
            let rule = rules
                .iter()
                .find(|(words, _)| words.iter().all(|w| joined.contains(w)));
            rule.map(|(_, out)| out.to_string())
                .ok_or(format!("no answer to: {joined}"))
        }
    }

    pub(crate) const URL: &str = "https://github.com/nha-org/rust-ci-mirror/commit/3d312b9c222ec23cae423a2450993ea13b5560ef/checks";

    /// The head's red as the status read answers it: its mirror commit's checks, then when it was written.
    pub(crate) fn status() -> String {
        format!("{URL} 2026-11-11T15:56:12Z")
    }

    #[test]
    fn a_mirror_rerun_gone_green_is_green_and_written_on_the_head() {
        let _budget = TestBudget::new();
        let runs = r#"[{"id":3,"name":"test","status":"completed","conclusion":"success"},
                       {"id":4,"name":"publish","status":"completed","conclusion":"skipped"}]"#;
        let status = status();
        let rules: [(&[&str], &str); 5] = [
            (&[&format!("commits/{HEAD}/pulls")], "1444"),
            (&[&format!("commits/{HEAD}/status")], &status),
            (&["check-runs"], runs),
            (&["POST", &format!("statuses/{HEAD}")], "{}"),
            // a short sha read whole: a status is written on the full one alone (/code-review of 0906afff9)
            (&["commits/d321d07d", ".sha"], HEAD),
        ];
        let mut calls = vec![];
        let mut gh = stand_in(&rules, &mut calls);
        assert_eq!(reread("d321d07d", &mut gh), Ok(Reread::Green));
        drop(gh);
        let post = calls
            .iter()
            .find(|c| c.contains(&"POST".to_string()))
            .expect("the head's status written");
        assert!(
            post.contains(&"state=success".to_string())
                && post.contains(&format!("target_url={URL}")),
            "{post:?}"
        );
    }

    #[test]
    fn a_red_ci_mirror_rust_shows_its_failing_steps_last_lines_on_the_pr() {
        let _budget = TestBudget::new();
        let head = format!("1444 {HEAD}");
        let runs = r#"[{"id":130627997259,"name":"test","status":"completed","conclusion":"failure",
                        "html_url":"https://github.com/nha-org/rust-ci-mirror/actions/runs/43051056804/job/130627997259"},
                       {"id":5,"name":"site-musl","status":"completed","conclusion":"success"}]"#;
        // as `gh run view --log-failed` printed job 130627997259 (2026-11-11), 45 lines of it
        let step = "Run sudo apt-get update -qq";
        let mut log: Vec<String> = (1..=44)
            .map(|i| {
                format!("test\t{step}\t2026-11-11T15:56:09.43{i:02}Z E: Failed to fetch line {i}")
            })
            .collect();
        log.push(format!("test\t{step}\t2026-11-11T15:56:09.4392754Z \u{1b}[31m##[error]Process completed with exit code 100.\u{1b}[0m"));
        let log = log.join("\n");
        let status = status();
        let rules: [(&[&str], &str); 6] = [
            (&["pr", "view", "1444"], &head),
            (&[&format!("commits/{HEAD}/status")], &status),
            (&["check-runs"], runs),
            (&["issues/1444/comments"], "an earlier comment\n"),
            (
                &[
                    "run",
                    "view",
                    "--job",
                    "130627997259",
                    "--log-failed",
                    "-R",
                    "nha-org/rust-ci-mirror",
                ],
                &log,
            ),
            (&["pr", "comment", "1444"], ""),
        ];
        let mut calls = vec![];
        let mut gh = stand_in(&rules, &mut calls);
        let read = reread("1444", &mut gh);
        drop(gh);
        assert_eq!(read, Ok(Reread::Red(None)), "a settled red stays red");
        let posted = calls
            .iter()
            .find(|c| c[..2] == ["pr", "comment"])
            .expect("posted on the PR");
        let body = &posted[4];
        println!("{body}");
        assert!(
            body.starts_with(
                "**ci-mirror/rust job 130627997259 is red** on d321d07d: `test` failed"
            ),
            "{body}"
        );
        assert!(
            body.contains(&format!("failed at step « {step} »")),
            "{body}"
        );
        assert!(
            body.contains("\n##[error]Process completed with exit code 100.\n````"),
            "{body}"
        );
        assert!(
            body.contains("\nE: Failed to fetch line 6\n") && !body.contains("line 5\n"),
            "the last 40 lines: {body}"
        );
        assert!(
            !body.contains("2026-11-11T15:56") && !body.contains('\u{1b}'),
            "{body}"
        );

        // posted once a job: a comment holding its marker posts nothing more
        let seen = format!("{}\n", body);
        let rules: [(&[&str], &str); 4] = [
            (&["pr", "view", "1444"], &head),
            (&[&format!("commits/{HEAD}/status")], &status),
            (&["check-runs"], runs),
            (&["issues/1444/comments"], &seen),
        ];
        let mut calls = vec![];
        let mut gh = stand_in(&rules, &mut calls);
        assert_eq!(reread("1444", &mut gh), Ok(Reread::Red(None)));
        drop(gh);
        assert!(
            !calls.iter().any(|c| c[..2] == ["pr", "comment"]),
            "{calls:?}"
        );
    }

    #[test]
    fn a_red_that_names_no_mirror_commit_or_a_run_older_than_it_stays_red() {
        let _budget = TestBudget::new();
        let head = format!("1444 {HEAD}");
        // CiMirror's refusal (a secret scan hit) names the mirror repository, no commit
        let refused = "https://github.com/nha-org/rust-ci-mirror 2026-11-11T15:56:12Z";
        let rules: [(&[&str], &str); 2] = [(&["pr", "view"], &head), (&["/status --jq"], refused)];
        let mut calls = vec![];
        let mut gh = stand_in(&rules, &mut calls);
        assert_eq!(reread("1444", &mut gh), Ok(Reread::Red(None)));
        drop(gh);
        assert_eq!(calls.len(), 2, "{calls:?}");
        // CiMirror's 24-hour give-up: the run it waited on still queued, started before the red was written
        let runs = r#"[{"id":6,"name":"test","status":"queued","conclusion":null,"started_at":"2026-11-10T15:00:00Z"}]"#;
        let status = status();
        let rules: [(&[&str], &str); 3] = [
            (&["pr", "view"], &head),
            (&["/status --jq"], &status),
            (&["check-runs"], runs),
        ];
        let mut calls = vec![];
        let mut gh = stand_in(&rules, &mut calls);
        assert_eq!(reread("1444", &mut gh), Ok(Reread::Red(None)));
    }

    #[test]
    fn a_green_the_head_did_not_take_is_no_verdict() {
        let _budget = TestBudget::new();
        let head = format!("1444 {HEAD}");
        let runs = r#"[{"id":3,"name":"test","status":"completed","conclusion":"success"}]"#;
        let status = status();
        let rules: [(&[&str], &str); 3] = [
            (&["pr", "view"], &head),
            (&["/status --jq"], &status),
            (&["check-runs"], runs),
        ];
        let mut calls = vec![];
        let mut gh = stand_in(&rules, &mut calls);
        assert!(reread("1444", &mut gh).is_err(), "the POST had no answer");
    }

    #[test]
    fn a_mirror_read_that_fails_is_an_error_never_the_old_red() {
        let _budget = TestBudget::new();
        let mut calls = vec![];
        let mut gh = stand_in(&[], &mut calls);
        assert!(reread("1444", &mut gh).is_err());
        drop(gh);
        assert_eq!(mirror_of(URL), Some(("nha-org/rust-ci-mirror", COMMIT)));
        assert_eq!(mirror_of("https://github.com/nha-org/rust-ci-mirror"), None);
    }
}
