//! `dash ci list|runner|checks|stale`, registered here through the core's `Command` (task rust-hostman-ci-wire moved them out
//! of the binary's match, which may not grow): each answers from this crate, the host handing in the repository.
use dash_core::{Command, Host};
use std::io::Write;

const LIST: &str = "dash ci list [--repo <owner/name>] [--from <day>] [--days <n>]";
const RUNNER: &str = "dash ci runner <folder> <owner/name> (self-hosted, hosted or unknown)";
const CHECKS: &str = "dash ci checks <pr|sha> [<owner/repo>]";
const STALE: &str = "dash ci stale <pr> [<owner/repo>]";

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

inventory::submit! {
    Command { usage: LIST, words: &["ci", "list"], run: Some(|host, rest, out| hosting(host, &[&["list"], rest].concat(), LIST, out)) }
}
inventory::submit! {
    Command { usage: RUNNER, words: &["ci", "runner"], run: Some(|host, rest, out| hosting(host, &[&["runner"], rest].concat(), RUNNER, out)) }
}
inventory::submit! {
    Command { usage: CHECKS, words: &["ci", "checks"], run: Some(|host, rest, out| {
        let [target, repo @ ..] = rest else { usage(CHECKS) };
        let here = std::env::current_dir().unwrap_or_default();
        let gh = crate::checks::gh_in("gh", &here);
        let rows = (repo.len() <= 1).then(|| crate::checks::checks(&gh, target, repo.first().copied(), &here, &host.repo()));
        said(rows.unwrap_or_else(|| usage(CHECKS)).map(|rows| serde_json::json!(rows)), out)
    }) }
}
inventory::submit! {
    Command { usage: STALE, words: &["ci", "stale"], run: Some(|_, rest, out| {
        let [pr, repo @ ..] = rest else { usage(STALE) };
        let here = std::env::current_dir().unwrap_or_default();
        let gh = crate::checks::gh_in("gh", &here);
        let stale = (repo.len() <= 1).then(|| crate::checks::stale(&gh, pr, repo.first().copied()));
        let json = stale.unwrap_or_else(|| usage(STALE));
        said(json.map(|(head, files)| serde_json::json!({"head": head, "files": files})), out)
    }) }
}
