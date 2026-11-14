//! `dash ci runner`: whether a repository's CI runs on a self-hosted runner of its own, the port of `Dashboard.CiBilling`'s
//! `runner?/1` (task ci-billing-runner-truth). The Elixir read only the workflows' runs-on, so replicator, whose runners API
//! lists none, read « self-hosted » and the PM's billing answer sent a reviewer the wrong way twice. A repo is self-hosted
//! only when a workflow asks `self-hosted` and its runners API lists one online.

/// Whether a runs-on in `yml` names `self-hosted`: its line and the lines indented under it (a flow list, a block list, a
/// `labels:` map; a whitespace-only line deeper than runs-on keeps it going), as the Elixir's regex read it.
pub fn asks_self_hosted(yml: &str) -> bool {
    let lines: Vec<&str> = yml.lines().collect();
    let indent = |l: &str| l.len() - l.trim_start().len();
    lines.iter().enumerate().any(|(at, line)| {
        if !line.trim_start().starts_with("runs-on:") {
            return false;
        }
        let own = indent(line);
        std::iter::once(*line)
            .chain((lines[at + 1..].iter().copied()).take_while(|l| indent(l) > own))
            .any(|l| l.contains("self-hosted"))
    })
}

/// Some(true) self-hosted, Some(false) hosted, None unknown (no workflows, or runners unread while a workflow asks for one:
/// the answer then stays the runner's, as the Elixir's nil did). `runners` counts the online ones its runners API lists, read
/// only when a workflow asks for one.
pub fn runner(workflows: &[String], runners: impl FnOnce() -> Option<usize>) -> Option<bool> {
    if workflows.is_empty() {
        return None;
    }
    if !workflows.iter().any(|w| asks_self_hosted(w)) {
        return Some(false);
    }
    runners().map(|n| n > 0)
}

/// The answer for the repo checked out at `folder`, on GitHub as `slug`, through `ask` (gh's words).
pub fn read(
    ask: &dyn Fn(&[&str]) -> Result<String, String>,
    folder: &std::path::Path,
    slug: &str,
) -> Option<bool> {
    let mut workflows = vec![];
    if let Ok(entries) = std::fs::read_dir(folder.join(".github/workflows")) {
        for e in entries.flatten() {
            let p = e.path();
            if matches!(p.extension().and_then(|x| x.to_str()), Some("yml" | "yaml")) {
                workflows.push(std::fs::read_to_string(&p).unwrap_or_default());
            }
        }
    }
    // the repo's own runners alone: nha-org's are Depot's dependabot machines, no self-hosted label (2026-11-14)
    let path = format!("repos/{slug}/actions/runners?per_page=100");
    let online = "[.runners[] | select(.status == \"online\")] | length";
    runner(&workflows, || {
        let count = ask(&["api", &path, "--jq", online]).ok()?;
        count.trim().parse().ok()
    })
}

/// The word `dash ci runner` prints.
pub fn word(answer: Option<bool>) -> &'static str {
    match answer {
        Some(true) => "self-hosted",
        Some(false) => "hosted",
        None => "unknown",
    }
}

/// `dash ci list|runner <arguments>`, answered here so the binary only gathers it: what to print, or why not (`usage` for
/// words it does not take). `repo` is the site_perso checkout, where `ci list` keeps its cache.
pub fn command(args: &[&str], repo: &std::path::Path) -> Result<String, String> {
    let ask = |w: &[&str]| dash_process::run("gh", w, None, std::time::Duration::from_secs(300));
    match args {
        ["runner", folder, slug] => Ok(format!(
            "{}\n",
            word(read(&ask, std::path::Path::new(folder), slug))
        )),
        ["list", rest @ ..] => crate::runs::command(rest, repo, &ask),
        _ => Err("usage".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// Done-when 1: replicator, a check.yml asking self-hosted and a runners API listing none, reads hosted.
    #[dash_test]
    fn replicator_with_no_runner_reads_hosted() {
        let tmp = dash_meter::TestDir::new("hosting");
        let dir = tmp.0.clone();
        std::fs::create_dir_all(dir.join(".github/workflows")).unwrap();
        let check = "jobs:\n  check:\n    runs-on:\n      - self-hosted\n      - replicator\n    steps: []\n";
        std::fs::write(dir.join(".github/workflows/check.yml"), check).unwrap();
        let none = |w: &[&str]| {
            assert_eq!(
                w[1],
                "repos/nha-org/replicator/actions/runners?per_page=100"
            );
            Ok("0\n".to_string())
        };
        assert_eq!(read(&none, &dir, "nha-org/replicator"), Some(false));
        let one = |_: &[&str]| Ok("1".to_string());
        assert_eq!(read(&one, &dir, "nha-org/replicator"), Some(true));
        let unread = |_: &[&str]| Err("gh did not answer".to_string());
        assert_eq!(read(&unread, &dir, "nha-org/replicator"), None);
    }

    #[dash_test]
    fn hosted_workflows_and_none() {
        let hosted =
            "jobs:\n  t:\n    runs-on: ubuntu-latest\n    env:\n      X: self-hosted\n".to_string();
        assert!(!asks_self_hosted(&hosted));
        assert_eq!(runner(&[hosted], || panic!("no runner asked")), Some(false));
        assert!(asks_self_hosted(
            "jobs:\n  t:\n    runs-on: [self-hosted, mac]\n"
        ));
        assert!(asks_self_hosted(
            "    runs-on:\n      labels: self-hosted\n"
        ));
        assert_eq!(runner(&[], || Some(1)), None);
        assert!(asks_self_hosted(
            "    runs-on:\n      \n      - self-hosted\n"
        ));
    }
}
