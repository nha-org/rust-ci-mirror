//! His private capture policy (task extension-capture-policy, rfc-chrome-extension section 11): which domains the extension
//! may take a HAR on, kept in the live folder outside git, since a HAR holds session cookies (@nha, 2026-11-02: « there needs
//! to be a private list of domains we capture too: and to what degree. -- ex. linear/slack/gmail -> capture enough to
//! replicate (HAR etc.) _maybe_ make it optional to start/stop capture ex. in case of bugs -- klar.im unsure -- all pages -
//! allow comments -- all pages - allow one-time screenshots + html capture »).
//! The file is `<data>/capture-policy.json`, `{"domains": {"<host>": "har" | "unset"}}`, written with his first list when
//! missing. Every page gets the every-page level; only a host named `har`, or a subdomain of one, adds the HAR. Any other
//! word, `unset` included, gives none; a file that does not parse is a 500.
use serde_json::{Value as Json, json};
use std::path::Path;

/// The policy's file, in the data folder.
pub const FILE: &str = "capture-policy.json";
/// What every page allows.
pub const EVERY_PAGE: [&str; 3] = ["comment", "screenshot", "html"];

/// His list of 2026-11-02: Linear, Slack and Gmail replicated, klar.im left for him to decide.
pub fn first() -> Json {
    json!({"domains": {
        "linear.app": "har",
        "slack.com": "har",
        "mail.google.com": "har",
        "klar.im": "unset",
    }})
}

/// The policy in `dir`, written with `first()` when missing: to a file of its own (pid and thread, since `dash serve` answers
/// each connection on its own thread), then linked in, which never replaces a file: a request at the same time never reads
/// it half written, and a policy he saved meanwhile is kept and read.
pub fn read(dir: &Path) -> Result<Json, String> {
    let file = dir.join(FILE);
    let fail = |e: std::io::Error| format!("{}: {e}", file.display());
    let text = match std::fs::read_to_string(&file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let text = serde_json::to_string_pretty(&first()).expect("json");
            let id = format!("{}.{:?}", std::process::id(), std::thread::current().id());
            let fresh = file.with_extension(format!(
                "json.{}",
                id.replace(|c: char| !c.is_ascii_alphanumeric(), "")
            ));
            std::fs::write(&fresh, &text).map_err(fail)?;
            let linked = std::fs::hard_link(&fresh, &file);
            let _ = std::fs::remove_file(&fresh);
            match linked {
                Ok(()) => text,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    std::fs::read_to_string(&file).map_err(fail)?
                }
                Err(e) => return Err(fail(e)),
            }
        }
        Err(e) => return Err(fail(e)),
    };
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))
}

/// A host Chrome's `*://*.<host>/*` pattern takes: labels of letters, digits and inner hyphens, at least two, the last no
/// number (an IP). Anything else (a port, a path, a `*`, a space) would make Chrome refuse the menu or widen it.
fn host(k: &str) -> bool {
    let labels: Vec<&str> = k.split('.').collect();
    let label = |l: &&str| {
        !l.is_empty()
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    };
    labels.len() > 1
        && labels.iter().all(label)
        && !labels
            .last()
            .is_some_and(|l| l.bytes().all(|b| b.is_ascii_digit()))
}

/// The hosts whose word is `word`, lowercased, once each. Two spellings of one host with different words: any word but
/// `har` wins, so a host is never both `har` and `unset`.
fn hosts(policy: &Json, word: &str) -> Vec<String> {
    let all = policy["domains"].as_object().cloned().unwrap_or_default();
    let words = |w: &dyn Fn(&Json) -> bool| -> std::collections::BTreeSet<String> {
        (all.iter())
            .filter(|(k, v)| w(v) && host(k))
            .map(|(k, _)| k.to_ascii_lowercase())
            .collect()
    };
    let not_har = words(&|v| v != "har");
    match word {
        "har" => words(&|v| v == "har")
            .difference(&not_har)
            .cloned()
            .collect(),
        _ => words(&|v| v == word).into_iter().collect(),
    }
}

/// What the extension reads: the every-page level, the HAR hosts (each with its subdomains, as Chrome's `*.` pattern), and
/// the hosts still unset. Any other word than `har`, `unset` included, gives no HAR.
pub fn served(policy: &Json) -> Json {
    json!({"every_page": EVERY_PAGE, "har": hosts(policy, "har"), "unset": hosts(policy, "unset")})
}

/// `GET /capture/policy`'s status, type and body for the data folder `dir`: a broken file is a 500, which the extension
/// reads as no change.
pub fn answer(dir: Result<&Path, String>) -> (&'static str, &'static str, String) {
    match dir.and_then(read) {
        Ok(p) => ("200 OK", "application/json", served(&p).to_string()),
        Err(e) => ("500 Internal Server Error", "text/plain; charset=utf-8", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::{TestDir, dash_test};

    #[dash_test]
    fn a_missing_policy_is_written_with_his_list_every_page_comments_and_captures_and_three_take_a_har()
     {
        let scratch = TestDir::new("capture-first");
        let dir = &scratch.0;
        let p = read(dir).expect("read");
        assert!(dir.join(FILE).exists(), "written in the live folder");
        let s = served(&p);
        assert_eq!(s["every_page"], json!(["comment", "screenshot", "html"]));
        assert_eq!(
            s["har"],
            json!(["linear.app", "mail.google.com", "slack.com"])
        );
        assert_eq!(read(dir).expect("again"), p, "kept, not rewritten");
        // a policy he saved before the first read is never replaced
        std::fs::write(dir.join(FILE), r#"{"domains": {"klar.im": "har"}}"#).expect("his");
        assert_eq!(served(&read(dir).expect("his"))["har"], json!(["klar.im"]));
        assert_eq!(
            std::fs::read_dir(dir).expect("dir").count(),
            1,
            "no temp file left"
        );
    }

    #[dash_test]
    fn klar_im_is_unset_every_page_level_never_a_har_until_he_sets_it() {
        let p = first();
        assert_eq!(p["domains"]["klar.im"], "unset");
        let s = served(&p);
        assert_eq!(s["unset"], json!(["klar.im"]));
        assert!(
            !s["har"]
                .as_array()
                .expect("har")
                .contains(&json!("klar.im"))
        );
        // he sets it: then, and only then, a HAR
        let mut set = p.clone();
        set["domains"]["klar.im"] = json!("har");
        assert!(
            served(&set)["har"]
                .as_array()
                .expect("har")
                .contains(&json!("klar.im"))
        );
    }

    #[dash_test]
    fn a_broken_file_another_word_or_a_key_that_is_no_host_never_gives_a_har() {
        let scratch = TestDir::new("capture-broken");
        let dir = &scratch.0;
        std::fs::write(dir.join(FILE), "{not json").expect("write");
        assert!(read(dir).is_err());
        let p = json!({"domains": {"a.com": "yes", "b.com": true, "c.com": "HAR", "*": "har", "x.com:3000": "har",
            "y.com/a": "har", "localhost": "har", "z .com": "har", "Linear.App": "har", "linear.app": "har",
            "Klar.IM": "unset", "1.2.3.4": "har", "-x.com": "har", "a-.com": "har", "Slack.com": "unset", "slack.com": "har"}, "har": ["d.com"]});
        let s = served(&p);
        assert_eq!(s["har"], json!(["linear.app"]), "lowercased, once");
        assert_eq!(
            s["unset"],
            json!(["klar.im", "slack.com"]),
            "any word but har wins"
        );
        assert_eq!(served(&json!({}))["har"], json!([]));
    }
}
