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

// the name its firings are recorded and listed under
use crate::firings::REVIEW_CLAIM as FIRING;

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
        let _ = crate::firings::record(file, FIRING);
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

static REVIEWS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(?:re-?)?review(?:s|ing)?\b").expect("review"));
static PR_REF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:github\.com/([\w.-]+/[\w.-]+)/pull/|\b(?:PR|pull request)\s*#?)(\d+)")
        .expect("pr")
});

/// The deny for an Agent call whose prompt reviews a PR nobody has claimed (task helper-review-claims; @nha, 2026-10-22: « No
/// double work. »: a helper reviewed PR 635 with no claim and the PM pass started a second reviewer). `repo` is the checkout's
/// `owner/name` for a bare « PR 12 »; `held` asks whether a link has a live claim, `dash review push` read as no one in
/// particular. A PR the prompt does not name, or one held, passes.
pub fn answer_agent(
    event: &Value,
    repo: &dyn Fn() -> Option<String>,
    held: &dyn Fn(&str) -> bool,
    firings: Option<&std::path::Path>,
) -> Option<Value> {
    let prompt = event.get("tool_input")?.get("prompt")?.as_str()?;
    if !REVIEWS.is_match(prompt) {
        return None;
    }
    // only the checkout's own repo is judged: `dash review push` holds nothing against another's key, so it would read unclaimed
    let here = repo()?;
    let mut links: Vec<String> = (PR_REF.captures_iter(prompt))
        .filter(|c| c.get(1).is_none_or(|m| m.as_str() == here))
        .map(|c| format!("https://github.com/{here}/pull/{}", &c[2]))
        .collect();
    links.sort();
    links.dedup();
    // every PR the prompt names must be claimed, the first unclaimed one is the deny
    let link = links.into_iter().find(|l| !held(l))?;
    if let Some(file) = firings {
        let _ = crate::firings::record(file, FIRING);
    }
    let reason = format!(
        "Refused by dash hook review-claim: this Agent reviews {link} and nobody holds a live claim on it, so the PM pass would \
         start a second reviewer (@nha, 2026-10-22: « No double work. »). Run `dash review claim {link} <who>` first, then start the helper."
    );
    Some(
        json!({"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "deny",
                "permissionDecisionReason": reason}}),
    )
}

/// `dash hook review-claim`: this module's `answer` and `answer_agent` with this checkout's origin and branch, and the app's
/// `dash review push` verdict asked as the pre-push hook asks it (DASH_BIN, else the project's private/dashboard/bin/dash).
pub fn run(out: &mut dyn std::io::Write) {
    use std::io::Read;
    use std::path::PathBuf;
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let Ok(event) = serde_json::from_str::<Value>(&input) else {
        return;
    };
    let here_dir = std::env::current_dir().unwrap_or_default();
    let cwd = event["cwd"]
        .as_str()
        .map_or(here_dir.clone(), PathBuf::from);
    let project = std::env::var_os("CLAUDE_PROJECT_DIR").map_or(here_dir, PathBuf::from);
    use std::time::Duration;
    let git = |args: &[&str]| {
        let mut c = std::process::Command::new("git");
        c.args(args)
            .current_dir(&cwd)
            .stdin(std::process::Stdio::null());
        let said = super::ran_within(&mut c, Some(Duration::from_secs(3)));
        // its answer is one line, the last: `ran` keeps stderr in the same file, where a warning comes first
        let line = said
            .said
            .lines()
            .last()
            .unwrap_or_default()
            .trim()
            .to_string();
        Some(line).filter(|o| said.ok() && !o.is_empty())
    };
    // the default branch is never asked about, as pre-push skips it (forge.env's DEFAULT_BRANCH, Ctx::main); four git calls
    // at 3 s and one ask at 12 s stay under the hook's 30 s timeout in .claude/settings.json (/code-review); an Agent's
    // prompt naming more PRs is cut at that timeout, which lets the call through
    let here = || {
        let url = git(&["remote", "get-url", "origin"]).unwrap_or_default();
        let repo = super::pre_push::github_repo(&url);
        let top = git(&["rev-parse", "--show-toplevel"]).map(PathBuf::from);
        let main = top.map(|t| super::default_branch(&t));
        let branch =
            git(&["symbolic-ref", "--short", "-q", "HEAD"]).filter(|b| Some(b) != main.as_ref());
        (repo, branch)
    };
    let app = std::env::var_os("DASH_BIN")
        .map_or_else(|| project.join("private/dashboard/bin/dash"), PathBuf::from);
    // `dash review push <key>`: None when the app is silent, else its refusal (None: free to push). `anon` asks as no session on
    // no pane, so a claim made from this one counts too
    let push = |key: &str, anon: bool| {
        let mut c = std::process::Command::new(&app);
        c.args(["review", "push", key]).current_dir(&cwd);
        c.env("DASH_TIMEOUT", "10").env("DASH_RESTART_TRIES", "0");
        // removed, never set empty: the older app may read an empty name as one (/code-review of PR 1691)
        if anon {
            c.env("DASH_SESSION", "nobody")
                .env_remove("REVIEW_CLAIM_WHO")
                .env_remove("HERDR_PANE_ID");
        }
        let said = super::ran_within(&mut c, Some(Duration::from_secs(12)));
        let refused = !said.ok() && said.said.contains("review push: refused");
        // only a clean answer is « free »: a timeout or a crash is silent, and the hook then lets the call through
        (said.ok() || refused).then(|| refused.then(|| said.said.trim().to_string()))
    };
    let ask = |key: &str| push(key, false).flatten();
    // an app that does not answer refuses nothing, and claims nothing is missing
    let held = |link: &str| !matches!(push(link, true), Some(None));
    let firings = crate::firings::log_from_env();
    let repo = || super::pre_push::github_repo(&git(&["remote", "get-url", "origin"])?);
    let answer = if event["tool_name"] == "Agent" {
        answer_agent(&event, &repo, &held, firings.as_deref())
    } else {
        answer(&event, &here, &ask, firings.as_deref())
    };
    if let Some(answer) = answer {
        let _ = writeln!(out, "{answer}");
    }
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

    /// Done-when 1: an Agent whose prompt reviews a PR with no live claim is refused, naming `dash review claim`; a held PR,
    /// a prompt that reviews nothing or names no PR runs.
    #[dash_test]
    fn an_agent_reviewing_an_unclaimed_pr_is_refused_naming_dash_review_claim() {
        let agent = |p: &str| json!({"tool_name": "Agent", "tool_input": {"prompt": p}});
        let repo = || Some("nha-org/site_perso".to_string());
        let seen = std::cell::RefCell::new(vec![]);
        let none = |k: &str| {
            seen.borrow_mut().push(k.to_string());
            false
        };
        let said = answer_agent(&agent("Review PR 635, post findings"), &repo, &none, None)
            .expect("denied");
        let why = said["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap_or_default();
        assert_eq!(said["hookSpecificOutput"]["permissionDecision"], "deny");
        let pr = "https://github.com/nha-org/site_perso/pull/635";
        assert!(why.contains(&format!("dash review claim {pr}")), "{why}");
        assert_eq!(seen.borrow()[0], pr);
        // another repo's link is not judged: `dash review push` holds nothing against it
        let link = "run /code-review on https://github.com/o/r/pull/9";
        assert!(answer_agent(&agent(link), &repo, &none, None).is_none());
        assert_eq!(seen.borrow().len(), 1);
        // « reviewing » and « re-review » are reviews too
        assert!(answer_agent(&agent("Start reviewing PR 635"), &repo, &none, None).is_some());
        assert!(answer_agent(&agent("Re-review PR 635"), &repo, &none, None).is_some());
        let held = |_: &str| true;
        assert!(answer_agent(&agent("Review PR 635"), &repo, &held, None).is_none());
        assert!(answer_agent(&agent("Fix PR 635"), &repo, &none, None).is_none());
        assert!(answer_agent(&agent("Review the diff"), &repo, &none, None).is_none());
        // the reviewer, or a bare « pull 3 commits », is no review of a PR
        assert!(
            answer_agent(
                &agent("Merge PR 12, the reviewer approved"),
                &repo,
                &none,
                None
            )
            .is_none()
        );
        assert!(
            answer_agent(
                &agent("Review the diff, pull 3 commits back"),
                &repo,
                &none,
                None
            )
            .is_none()
        );
        // every PR named counts: 635 claimed, 640 not
        let only_635 = |k: &str| k.ends_with("/635");
        let said = answer_agent(&agent("Review PR 635 and PR 640"), &repo, &only_635, None)
            .expect("denied");
        assert!(
            said["hookSpecificOutput"]["permissionDecisionReason"]
                .as_str()
                .unwrap_or_default()
                .contains("claim https://github.com/nha-org/site_perso/pull/640")
        );
    }
}
