//! `dash ci health`: the warnings of main's green runs, each workflow's newest of the last week against its newest of the week
//! before (task ci-health-weekly; @nha, 2026-10-31: « We should periodically review this. And see if CI should be made faster
//! too »). The port of `Dashboard.CiHealth`, which read dashboard.yml alone: that workflow left with the Elixir app on
//! 2026-11-16, and the report stayed on its 11-08 run (task ci-runner-cache-restore). Every workflow with a green run on main
//! that week is read, so a new one needs no list here.
//!
//! A kind is GitHub's annotation, warning or failure (a checkout's « The repository will be recreated », a cache's failed
//! tar), as `actions: <first sentence>`: what the old report read from `##[warning]` log lines, without downloading the logs.
//! Its advisories and slowest tests read mix's output, which no workflow prints any more (rust/CLAUDE.md rule 9: a check of
//! Elixir files alone is not ported).
// ponytail: GitHub's runs listing gives 100 a page, read whole with --paginate; a week of main's green runs is under 200.
use std::collections::BTreeMap;

/// gh with its words: stdout, or why not.
pub type Ask<'a> = &'a dyn Fn(&[&str]) -> Result<String, String>;

/// One green run on main.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub id: u64,
    pub workflow: String,
    pub url: String,
    pub created: String,
    pub sha: String,
}

/// Each workflow's newest green run on main created in days `from` to `to` (UTC, both included).
pub fn newest(ask: Ask, slug: &str, from: i64, to: i64) -> Result<BTreeMap<String, Run>, String> {
    let path = format!(
        "repos/{slug}/actions/runs?branch=main&status=success&per_page=100&created={}..{}",
        dash_core::date(from),
        dash_core::date(to)
    );
    // a pull request's run from a branch named main is no run of main (/code-review)
    let jq = ".workflow_runs[] | select(.event != \"pull_request\") | [.id, .path, .html_url, .created_at, .head_sha] | @tsv";
    let mut by = BTreeMap::new();
    for row in ask(&["api", "--paginate", &path, "--jq", jq])?.lines() {
        if let [id, path, url, created, sha] = row.split('\t').collect::<Vec<_>>()[..]
            && let Ok(id) = id.parse()
        {
            let workflow = path.rsplit('/').next().unwrap_or(path).to_string();
            let run = Run {
                id,
                workflow: workflow.clone(),
                url: url.into(),
                created: created.into(),
                sha: sha.chars().take(9).collect(),
            };
            // newest by its creation, whatever order the pages came in
            if by
                .get(&workflow)
                .is_none_or(|r: &Run| r.created < run.created)
            {
                by.insert(workflow, run);
            }
        }
    }
    Ok(by)
}

/// The kind of one annotation's message: `actions: ` and its first sentence, at most 80 characters, each word holding a digit
/// (a sha, a count, a run) read as `<n>`, so the same warning on another commit is the same kind, never NEW each week.
pub fn kind(message: &str) -> String {
    let first = message.split(['\n', '\r']).next().unwrap_or("");
    let first = first.split(". ").next().unwrap_or("");
    let words: Vec<&str> = (first.split_whitespace())
        .map(|w| {
            if w.chars().any(|c| c.is_ascii_digit()) {
                "<n>"
            } else {
                w
            }
        })
        .collect();
    format!(
        "actions: {}",
        words.join(" ").chars().take(80).collect::<String>()
    )
}

/// A run's warning and failure annotations, every job's: each kind with its count and one example message.
pub fn kinds(ask: Ask, slug: &str, run: u64) -> Result<Vec<(String, usize, String)>, String> {
    let jobs = format!("repos/{slug}/actions/runs/{run}/jobs?per_page=100");
    let mut by: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for job in ask(&["api", "--paginate", &jobs, "--jq", ".jobs[].id"])?.lines() {
        let path = format!(
            "repos/{slug}/check-runs/{}/annotations?per_page=100",
            job.trim()
        );
        let jq = r#".[] | select(.annotation_level != "notice") | .message | @json"#;
        for line in ask(&["api", "--paginate", &path, "--jq", jq])?.lines() {
            let message: String = serde_json::from_str(line).unwrap_or_else(|_| line.to_string());
            let e = by.entry(kind(&message)).or_insert((0, String::new()));
            e.0 += 1;
            if e.1.is_empty() {
                e.1 = message.trim().chars().take(200).collect();
            }
        }
    }
    let mut rows: Vec<_> = by.into_iter().map(|(k, (n, e))| (k, n, e)).collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    Ok(rows)
}

/// The report for the week ending on day `today` (UTC), through `ask`.
pub fn report(ask: Ask, slug: &str, today: i64) -> Result<String, String> {
    let now = newest(ask, slug, today - 6, today)?;
    // the week before unread still prints this week's, nothing marked NEW (/code-review)
    let (before, before_unread) = match newest(ask, slug, today - 13, today - 7) {
        Ok(by) => (by, None),
        Err(why) => (BTreeMap::new(), Some(why)),
    };
    if now.is_empty() {
        return Err(format!(
            "ci health: no green run on main from {}",
            dash_core::date(today - 6)
        ));
    }
    let mut lines = vec![format!(
        "ci health: main's newest green run of each workflow from {} (UTC), against its newest of the week before",
        dash_core::date(today - 6)
    )];
    if let Some(why) = before_unread {
        lines.push(format!(
            "the week before's runs unread, nothing marked NEW: {why}"
        ));
    }
    let (mut new, mut unread) = (0, 0);
    for (workflow, run) in &now {
        let prior = before.get(workflow);
        let vs = prior.map_or("no green run the week before".into(), |p| {
            format!("against {}", p.url)
        });
        lines.push(format!(
            "\n{workflow}: {} ({}, {}), {vs}",
            run.url, run.created, run.sha
        ));
        // one run gh cannot read is said on its workflow's line, and the others still read (/code-review)
        let was: Option<BTreeMap<String, usize>> = match prior.map(|p| kinds(ask, slug, p.id)) {
            Some(Ok(rows)) => Some(rows.into_iter().map(|(k, n, _)| (k, n)).collect()),
            Some(Err(why)) => {
                lines.push(format!(
                    "  the week before unread, nothing marked NEW: {why}"
                ));
                None
            }
            None => None,
        };
        let rows = match kinds(ask, slug, run.id) {
            Ok(rows) => rows,
            Err(why) => {
                unread += 1;
                lines.push(format!("  unread: {why}"));
                continue;
            }
        };
        if rows.is_empty() {
            lines.push("  none".into());
        }
        for (k, n, example) in rows {
            // no run the week before marks nothing new: every row would be
            let mark = match was.as_ref().map(|w| w.get(&k)) {
                Some(None) => {
                    new += 1;
                    "NEW "
                }
                _ => "",
            };
            let last = match was.as_ref().and_then(|w| w.get(&k)) {
                Some(n) => format!(" ({n} last week)"),
                None => String::new(),
            };
            lines.push(format!("  {mark}{k}: {n}{last}, e.g. {example}"));
        }
    }
    lines.push(format!(
        "\nnew: {new} rows marked NEW, each a task or a line on an open one"
    ));
    if unread > 0 {
        lines.push(format!("unread: {unread} workflows, their kinds unknown"));
    }
    // no workflow read is no report: « new: 0 » would read as a clean week (/code-review)
    if unread == now.len() {
        return Err(lines.join("\n"));
    }
    Ok(lines.join("\n") + "\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// gh answered from a table of path fragments, each call checked to name one.
    fn stand_in(
        answers: &'static [(&'static str, &'static str)],
    ) -> impl Fn(&[&str]) -> Result<String, String> {
        move |w: &[&str]| {
            let path = w[2];
            (answers
                .iter()
                .find(|(k, _)| path.contains(k))
                .map(|(_, v)| v.to_string()))
            .ok_or(format!("no answer for {path}"))
        }
    }

    /// Done-when 1 of ci-runner-cache-restore: the report reads the live workflows, never only dashboard.yml, names each run's
    /// link, and lists a checkout's recreate warning as an `actions:` kind, NEW against the week before.
    #[dash_test]
    fn each_workflow_s_newest_green_run_is_read_with_its_warnings() {
        // day 20411 is 2025-11-19: this week is 11-13..11-19, the week before 11-06..11-12
        let ask = stand_in(&[
            (
                "created=2025-11-13..2025-11-19",
                "11\t.github/workflows/site.yml\thttps://x/runs/11\t2025-11-18T10:00:00Z\tabcdef0123456\n\
                 12\t.github/workflows/site.yml\thttps://x/runs/12\t2025-11-19T10:00:00Z\tbcdef01234567\n\
                 13\t.github/workflows/superapp.yml\thttps://x/runs/13\t2025-11-19T09:00:00Z\tcdef012345678\n",
            ),
            (
                "created=2025-11-06..2025-11-12",
                "5\t.github/workflows/site.yml\thttps://x/runs/5\t2025-11-10T10:00:00Z\tdef0123456789\n",
            ),
            ("runs/12/jobs", "112\n"),
            ("runs/13/jobs", "113\n"),
            ("runs/5/jobs", "105\n"),
            (
                "check-runs/112/",
                "\"Unable to clean or reset the repository. The repository will be recreated instead.\"\n\
                 \"could not fetch 1962 from promisor remote\"\n",
            ),
            ("check-runs/113/", ""),
            (
                "check-runs/105/",
                "\"could not fetch 0000 from promisor remote\"\n",
            ),
        ]);
        let said = report(&ask, "o/r", 20411).unwrap();
        assert_eq!(
            said,
            "ci health: main's newest green run of each workflow from 2025-11-13 (UTC), against its newest of the week before\n\
             \n\
             site.yml: https://x/runs/12 (2025-11-19T10:00:00Z, bcdef0123), against https://x/runs/5\n  \
             NEW actions: Unable to clean or reset the repository: 1, e.g. Unable to clean or reset the repository. The repository will be recreated instead.\n  \
             actions: could not fetch <n> from promisor remote: 1 (1 last week), e.g. could not fetch 1962 from promisor remote\n\
             \n\
             superapp.yml: https://x/runs/13 (2025-11-19T09:00:00Z, cdef01234), no green run the week before\n  none\n\
             \n\
             new: 1 rows marked NEW, each a task or a line on an open one\n"
        );
        // a run gh cannot read is said on its line, and the report still prints
        let cut = stand_in(&[
            (
                "created=2025-11-13..2025-11-19",
                "12\t.github/workflows/site.yml\thttps://x/runs/12\t2025-11-19T10:00:00Z\tbcdef01234567\n\
                 13\t.github/workflows/superapp.yml\thttps://x/runs/13\t2025-11-19T09:00:00Z\tcdef012345678\n",
            ),
            ("runs/12/jobs", ""),
        ]);
        let said = report(&cut, "o/r", 20411).unwrap();
        // the week before's listing failed: said once, this week's still read
        assert!(
            said.contains("the week before's runs unread, nothing marked NEW: no answer for"),
            "{said}"
        );
        assert!(said.contains("site.yml: https://x/runs/12 (2025-11-19T10:00:00Z, bcdef0123), no green run the week before\n  none"), "{said}");
        assert!(said.contains("superapp.yml: https://x/runs/13"), "{said}");
        assert!(
            said.contains("  unread: no answer for repos/o/r/actions/runs/13/jobs"),
            "{said}"
        );
        assert!(
            said.ends_with("unread: 1 workflows, their kinds unknown\n"),
            "{said}"
        );
        // no workflow read at all is an error, never a clean week
        let dark = stand_in(&[(
            "created=2025-11-13..2025-11-19",
            "13\t.github/workflows/superapp.yml\thttps://x/runs/13\t2025-11-19T09:00:00Z\tcdef012345678\n",
        )]);
        let why = report(&dark, "o/r", 20411).unwrap_err();
        assert!(
            why.ends_with("unread: 1 workflows, their kinds unknown"),
            "{why}"
        );
        let none = stand_in(&[("created=", "")]);
        assert_eq!(
            report(&none, "o/r", 20411),
            Err("ci health: no green run on main from 2025-11-13".into())
        );
    }
}
