//! `dash ci list|runner|health|checks|stale|readings|wait`, registered here through the core's `Command` (task rust-hostman-ci-wire moved them out
//! of the binary's match, which may not grow): each answers from this crate, the host handing in the repository.
use dash_core::{Command, Host};
use std::io::Write;
use std::path::Path;

const LIST: &str = "dash ci list [--repo <owner/name>] [--from <day>] [--days <n>]";
const RUNNER: &str = "dash ci runner <folder> <owner/name> (self-hosted, hosted or unknown)";
const HEALTH: &str =
    "dash ci health (each workflow's warnings on main's green runs, this week against the last)";
const CHECKS: &str = "dash ci checks <pr|sha> [<owner/repo>] (a short sha too)";
const STALE: &str = "dash ci stale <pr> [<owner/repo>]";
// bin/dash runs this binary for it when the line is in it: keep its first words there in step
const WAIT: &str = "dash ci wait <pr|sha> (once a minute; exit 0 green, 1 red, 3 no verdict: a superseded cancel, a billing refusal or an infra red is never the verdict)";
const READINGS: &str =
    "dash ci readings (on a CI box: the runner's memory, the swap, the free disk)";

/// Its usage on stderr, exit 2, as the binary's usage did.
fn usage(line: &str) -> ! {
    eprintln!("usage: {line}");
    std::process::exit(2)
}

/// `dash ci list|runner`: where CI's jobs ran (task rust-ci-off-mac); whether a repo runs on its own runner (task
/// ci-billing-runner-truth).
fn hosting(host: &dyn Host, words: &[&str], line: &str, out: &mut dyn Write) {
    match crate::hosting::command(words, &host.repo()) {
        Ok(said) => write!(out, "{said}").unwrap_or_default(),
        Err(why) if why == "usage" => usage(line),
        Err(why) => {
            eprintln!("{why}");
            std::process::exit(1);
        }
    }
}

/// A head's checks or a PR's stale files as a merge reads them, as JSON; exit 3 when gh does not answer (task
/// rust-hooks-rest-port).
fn said(json: Option<serde_json::Value>, out: &mut dyn Write) {
    match json {
        Some(json) => writeln!(out, "{json}").unwrap_or_default(),
        None => {
            eprintln!("gh did not answer");
            std::process::exit(3);
        }
    }
}

dash_core::submit! {
    Command { usage: LIST, words: &["ci", "list"], run: Some(|host, rest, out| hosting(host, &[&["list"], rest].concat(), LIST, out)) }
}
// main's green runs of the week, each workflow's warnings against the week before (task ci-runner-cache-restore)
dash_core::submit! {
    Command { usage: HEALTH, words: &["ci", "health"], run: Some(|_, rest, out| {
        if !rest.is_empty() { usage(HEALTH) }
        let ask = |w: &[&str]| dash_process::run("gh", w, None, std::time::Duration::from_secs(120));
        let today = dash_process::now_s() as i64 / 86400;
        match crate::health::report(&ask, "nha-org/site_perso", today) {
            Ok(said) => write!(out, "{said}").unwrap_or_default(),
            Err(why) => {
                eprintln!("{why}");
                std::process::exit(1);
            }
        }
    }) }
}
dash_core::submit! {
    Command { usage: RUNNER, words: &["ci", "runner"], run: Some(|host, rest, out| hosting(host, &[&["runner"], rest].concat(), RUNNER, out)) }
}
dash_core::submit! {
    Command { usage: CHECKS, words: &["ci", "checks"], run: Some(|host, rest, out| {
        let ([target] | [target, _]) = rest else { usage(CHECKS) };
        let repo = rest.get(1).copied();
        let here = std::env::current_dir().unwrap_or_default();
        let gh = crate::checks::gh_in("gh", &here);
        // a short sha made whole as dash ci wait makes it (task rust-ci-wait-port)
        let target = crate::checks::whole_sha(&gh, target, repo, &here);
        let rows = crate::checks::checks(&gh, &target, repo, &here, &host.repo());
        said(rows.map(|rows| serde_json::json!(rows)), out)
    }) }
}
// a run's last step on a Hostman box (superapp.yml): the runner's memory and the box's disk, never a failing exit
dash_core::submit! {
    Command { usage: READINGS, words: &["ci", "readings"], run: Some(|_, rest, out| {
        if !rest.is_empty() { usage(READINGS) }
        let home = std::env::var("HOME").unwrap_or("/".into());
        let df = dash_process::run("df", &["-Pm", &home], None, std::time::Duration::from_secs(10)).unwrap_or_default();
        let runner = std::env::var("RUNNER_NAME").unwrap_or("an unnamed runner".into());
        writeln!(out, "{}", crate::readings::readings(Path::new("/"), &runner, &df)).unwrap_or_default()
    }) }
}
dash_core::submit! {
    Command { usage: STALE, words: &["ci", "stale"], run: Some(|_, rest, out| {
        let [pr, repo @ ..] = rest else { usage(STALE) };
        let here = std::env::current_dir().unwrap_or_default();
        let gh = crate::checks::gh_in("gh", &here);
        let stale = (repo.len() <= 1).then(|| crate::checks::stale(&gh, pr, repo.first().copied()));
        let json = stale.unwrap_or_else(|| usage(STALE));
        said(json.map(|(head, files)| serde_json::json!({"head": head, "files": files})), out)
    }) }
}
// the one CI wait, in this process: the older app's answered only while :7788 did (task rust-ci-wait-port)
dash_core::submit! {
    Command { usage: WAIT, words: &["ci", "wait"], run: Some(|host, rest, out| {
        // a flag is no PR: usage at once, never a wait to the deadline (/code-review: `dash ci wait --help`)
        let [target] = rest else { usage(WAIT) };
        if target.starts_with('-') { usage(WAIT) }
        let here = std::env::current_dir().unwrap_or_default();
        let (code, said, why) = crate::wait_head(&host.repo(), &here, target, || true);
        let _ = write!(out, "{said}");
        let _ = out.flush();
        eprint!("{why}");
        std::process::exit(code)
    }) }
}

#[cfg(test)]
mod tests {
    /// bin/dash hands `ci wait` to this binary only when the binary holds the words bin/dash greps for, WAIT's start: else it
    /// asks :7788, which a down app leaves unanswered (task rust-ci-wait-port). The case is bash's: the first pattern that
    /// matches « ci wait » decides, `"ci wait")` as much as PR 1868's `"ci "*)`.
    #[dash_meter::dash_test]
    fn bin_dash_routes_ci_wait_to_this_binary() {
        let known = "dash ci wait <pr|sha> (";
        assert!(super::WAIT.starts_with(known), "{}", super::WAIT);
        let Some(top) = dash_meter::monorepo() else {
            return;
        };
        let wrapper =
            std::fs::read_to_string(top.join("private/dashboard/bin/dash")).expect("bin/dash");
        let line = dash_meter::case_line(&wrapper, "ci wait");
        assert!(
            line.is_some_and(|l| l.contains(&format!("known='{known}'"))),
            "bin/dash does not route ci wait to Rust: its first case matching « ci wait » ({line:?}) lacks known='{known}', WAIT's start"
        );
    }
}
