//! `dash hook review-claim`, Claude's PreToolUse hook on Skill: a review pass (/code-review, /simplify, ponytail-review) is
//! refused before it starts when this session could not push its fixes, the PR handed over or held by another session
//! (task review-diff-merge-base; @nha, 2026-11-09: « every review pass runs before the claim is handed over, since a push
//! after it is refused. »). The verdict is the pre-push hook's own, `dash review push` (pre_push::review_push), so the two
//! never disagree; an app that does not answer refuses nothing. The PR is the one the skill's words name, else the
//! checkout's branch; a detached head or the default branch naming none is not judged.
use regex::Regex;
use serde_json::{Value, json};
use std::sync::LazyLock;

/// The skills that read a PR's diff to fix it: the passes whose fixes need a push (`ponytail:ponytail` is the PR lane's step 3).
pub const PASSES: [&str; 4] = [
    "code-review",
    "simplify",
    "ponytail:ponytail",
    "ponytail:ponytail-review",
];

static LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"https?://github\.com/[\w.-]+/[\w.-]+/pull/\d+").expect("link"));

/// What `dash review push` is asked about: the PR link the skill's words name, else `<repo>:<branch>`, the key the pre-push
/// hook asks with. `here` gives no branch on a detached head or on the default branch, which pre-push never asks about. A
/// bare number is never read as a PR: a short SHA or « the last 3 commits » would name another (/code-review).
pub fn key(args: &str, here: &dyn Fn() -> (Option<String>, Option<String>)) -> Option<String> {
    if let Some(link) = LINK.find(args) {
        return Some(link.as_str().to_string());
    }
    match here() {
        (Some(repo), Some(branch)) => Some(format!("{repo}:{branch}")),
        _ => None,
    }
}

/// The hook's deny for `event`, or None to let the skill run. `here` reads the checkout's (origin repo, branch), only once a
/// pass is asked for; `ask` is `dash review push <key>`'s refusal, None when it may push or the app is silent. Each deny is a
/// firing, `review_claim` in `firings`, which `dash gate firings` counts.
pub fn answer(
    event: &Value,
    here: &dyn Fn() -> (Option<String>, Option<String>),
    ask: &dyn Fn(&str) -> Option<String>,
    firings: Option<&std::path::Path>,
) -> Option<Value> {
    let input = event.get("tool_input")?;
    let skill = input.get("skill")?.as_str()?.trim_start_matches('/');
    if !PASSES.contains(&skill) {
        return None;
    }
    let args = input
        .get("args")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let why = ask(&key(args, here)?)?;
    if let Some(file) = firings {
        let _ = crate::firings::record(file, "review_claim");
    }
    let reason = format!(
        "Refused by dash hook review-claim: /{skill} now could not land its fixes, since a push from this session is refused: \
         {why} Every review pass runs before the claim is handed over (@nha, 2026-11-09: « every review pass runs before the \
         claim is handed over, since a push after it is refused. »): send what you found to its holder instead."
    );
    Some(
        json!({"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "deny",
                "permissionDecisionReason": reason}}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    fn skill(name: &str, args: &str) -> Value {
        json!({"tool_name": "Skill", "tool_input": {"skill": name, "args": args}})
    }

    fn here() -> (Option<String>, Option<String>) {
        (Some("nha-org/site_perso".into()), Some("task/x".into()))
    }

    #[dash_test]
    fn the_pr_asked_about_is_the_link_else_the_branch() {
        let url = "https://github.com/nha-org/site_perso/pull/12";
        let on = |b: Option<&'static str>| {
            move || (Some("nha-org/site_perso".to_string()), b.map(String::from))
        };
        let args = format!("medium {url}");
        assert_eq!(key(&args, &on(Some("task/x"))).as_deref(), Some(url));
        assert_eq!(key(&args, &on(None)).as_deref(), Some(url));
        assert_eq!(
            key("medium", &on(Some("task/x"))).as_deref(),
            Some("nha-org/site_perso:task/x")
        );
        assert_eq!(key("high 1234567", &on(None)), None, "a number is no PR");
    }

    /// The pass after the hand-over is denied before it starts, naming why; one this session may push runs, and so does
    /// any other skill or an app that does not answer.
    #[dash_test]
    fn a_review_pass_after_the_hand_over_is_refused_naming_why() {
        let held =
            "review push: refused: merge-on-green (pane w1:p2) holds the PR of branch task/x.";
        let refused = |k: &str| (k == "nha-org/site_perso:task/x").then(|| held.to_string());
        let said = answer(&skill("code-review", "medium"), &here, &refused, None).expect("denied");
        let h = &said["hookSpecificOutput"];
        assert_eq!(h["permissionDecision"], "deny");
        let why = h["permissionDecisionReason"].as_str().unwrap_or_default();
        assert!(
            why.starts_with("Refused by dash hook review-claim: /code-review"),
            "{why}"
        );
        assert!(
            why.contains(held) && why.contains("before the claim is handed over"),
            "{why}"
        );
        assert!(answer(&skill("simplify", ""), &here, &refused, None).is_some());
        assert!(answer(&skill("ponytail:ponytail", ""), &here, &refused, None).is_some());
        let free = |_: &str| None;
        assert!(answer(&skill("code-review", "medium"), &here, &free, None).is_none());
        assert!(answer(&skill("humanizer", ""), &here, &refused, None).is_none());
    }

    #[dash_test]
    fn each_deny_is_counted_as_a_firing() {
        let dir = std::env::temp_dir().join(format!(
            "review-claim-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
        ));
        let file = dir.join("gate-firings.tsv");
        let refused = |_: &str| Some("held.".to_string());
        assert!(
            answer(
                &skill("code-review", "medium"),
                &here,
                &refused,
                Some(&file)
            )
            .is_some()
        );
        let text = std::fs::read_to_string(&file).expect("the firings");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(text.starts_with("review_claim\t"), "{text}");
    }
}
