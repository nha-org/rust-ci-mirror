//! `dash ci list|runner|health|checks|stale|readings`, registered here through the core's `Command` (task rust-hostman-ci-wire moved them out
//! of the binary's match, which may not grow): each answers from this crate, the host handing in the repository.
use dash_core::{Command, Host};
use std::io::Write;
use std::path::Path;

const LIST: &str = "dash ci list [--repo <owner/name>] [--from <day>] [--days <n>]";
const RUNNER: &str = "dash ci runner <folder> <owner/name> (self-hosted, hosted or unknown)";
const HEALTH: &str =
    "dash ci health (each workflow's warnings on main's green runs, this week against the last)";
const CHECKS: &str = "dash ci checks <pr|sha> [<owner/repo>]";
const STALE: &str = "dash ci stale <pr> [<owner/repo>]";
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
        let [target, repo @ ..] = rest else { usage(CHECKS) };
        let here = std::env::current_dir().unwrap_or_default();
        let gh = crate::checks::gh_in("gh", &here);
        let rows = (repo.len() <= 1).then(|| crate::checks::checks(&gh, target, repo.first().copied(), &here, &host.repo()));
        said(rows.unwrap_or_else(|| usage(CHECKS)).map(|rows| serde_json::json!(rows)), out)
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
