//! `dash review push <owner/repo:branch | PR link>`, the pre-push hook's question, judged first from Rust's own runs and holds
//! (task rust-forwarded-next, part 3; @nha, 2026-10-22: « No double work. »): a push to a PR a Rust run holds is refused,
//! each holder named with its pane, in `Dashboard.Orch.PM.push_refusal/3`'s words; the pusher is named by its environment,
//! as there. A push Rust holds nothing against goes on to the older app, which judges its own claims (`dash review claim`,
//! merge on green's pushers, a `dash task pr` owner): it keeps their lapses in its memory, which no file holds.
//! ponytail: the lane's hold and the orchestrator's hold no push, as the lane's reviewer pushes its fixes; a holder's stored
//! pane names it and never exempts a pusher, since herdr gives a dead run's pane to the next session: a Rust run is
//! exempt by its DASH_SESSION.
//! A task's own run holds no push, as `push_refusal/3` never counted it (second review of PR 1604): its row is claimed and
//! its `run <id>` hold stands for its whole landing, while its own session (`DASH_SESSION=rust-<id>`, the launcher's mark),
//! its run's update from main (no session named) and its reviewers push to its branch. Only a reviewer's run holds one.
use dash_core::tasks::{Hold, Standing};

/// A session holding the PR: its name (a task id, `review-<id>`) and the herdr pane it works in.
#[derive(Debug, PartialEq)]
pub struct Holder {
    pub name: String,
    pub pane: Option<String>,
}

/// `key`'s repository and branch, as `repo_branch/1` reads « owner/repo:branch »; else the key itself, a PR link.
pub fn repo_branch(key: &str) -> (Option<&str>, &str) {
    static SHAPED: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^([\w.-]+/[\w.-]+):(.+)$").expect("repo_branch's shape")
    });
    match SHAPED.captures(key) {
        Some(c) => (
            c.get(1).map(|m| m.as_str()),
            c.get(2).map_or(key, |m| m.as_str()),
        ),
        None => (None, key),
    }
}

/// The sessions holding the PR among Rust's rows and holds: a claimed run of the branch's review (`review-<id>`), and a
/// reviewer's run holding one of `prs`, the PRs the key names. Never the task's own run.
pub fn holders(
    branch: Option<&str>,
    prs: &[u64],
    rows: &[Standing],
    holds: &[Hold],
) -> Vec<Holder> {
    let pane = |id: &str| {
        rows.iter()
            .find(|r| r.id == id)
            .and_then(|r| r.pane.clone())
    };
    let claimed = |id: &str| rows.iter().any(|r| r.id == id && r.status == "claimed");
    let mut out: Vec<Holder> = vec![];
    let mut add = |name: String| {
        if !out.iter().any(|h| h.name == name) {
            let pane = pane(&name);
            out.push(Holder { name, pane });
        }
    };
    if let Some(id) = branch.and_then(|b| b.strip_prefix("task/")) {
        let name = format!("review-{id}");
        if claimed(&name) {
            add(name);
        }
    }
    for h in holds.iter().filter(|h| prs.contains(&h.pr)) {
        let run = h.holder.strip_prefix("run ");
        if let Some(id) = run.filter(|id| id.starts_with("review-")) {
            add(id.to_string());
        }
    }
    out
}

/// Why this session may not push `key`, None when Rust holds nothing against it: `push_refusal/3`. The pusher is
/// `DASH_SESSION` when the branch is its own `task/<id>`, else `REVIEW_CLAIM_WHO` or `DASH_SESSION`; a holder that is the
/// pusher holds nothing against it. A session the Rust launcher started is marked `rust-<id>` (`dash_launcher`'s command):
/// it is `<id>` here.
pub fn refusal(
    key: &str,
    env: &dyn Fn(&str) -> Option<String>,
    holders: &[Holder],
) -> Option<String> {
    let set = |k: &str| env(k).filter(|v| !v.is_empty());
    let branch = repo_branch(key).1;
    let is = |s: &String, id: &str| s == id || s.strip_prefix("rust-") == Some(id);
    let session = set("DASH_SESSION");
    let own = session
        .as_ref()
        .is_some_and(|s| branch.strip_prefix("task/").is_some_and(|id| is(s, id)));
    let who = if own {
        session
    } else {
        set("REVIEW_CLAIM_WHO").or(session)
    };
    let others: Vec<&Holder> = holders
        .iter()
        .filter(|h| !who.as_ref().is_some_and(|w| is(w, &h.name)))
        .collect();
    let first = others.first()?;
    let named: Vec<String> = (others.iter())
        .map(|h| match &h.pane {
            Some(p) => format!("{} (pane {p})", h.name),
            None => format!("{} (no live pane)", h.name),
        })
        .collect();
    let (named, to) = (named.join(", "), first.pane.as_ref().unwrap_or(&first.name));
    Some(match own {
        true => format!(
            "{named} holds your PR (branch {branch}: its reviewer, or the owner dash task pr named) and is its one pusher. Push \
             nothing: send it your fix (dash pane say {to} <text>), or wait for its end."
        ),
        false => format!(
            "{named} holds the PR of branch {branch} (a live review claim, merge on green's reviewer, the PM check's reviewer at \
             work, or the owner dash task pr named). Let it finish, or send it your change (dash pane say {to} <text>); its own \
             session pushes as it is, named by its DASH_SESSION or REVIEW_CLAIM_WHO."
        ),
    })
}

/// What `answer` reads: the PRs the key names in Rust's repository and its branch (None for another repository's key; a
/// link's branch as the PR mirror names it), then Rust's rows, a claim whose run is gone no longer `claimed`, and live holds.
pub type Read = (Option<(Vec<u64>, Option<String>)>, Vec<Standing>, Vec<Hold>);

/// `dash review push` as `Dashboard.CLI` answered it, `(exit, stdout, stderr)`, when Rust refuses it or it is misused; None
/// for a push Rust holds nothing against, which the older app judges. `read` takes the key's repository and the rest.
pub fn answer(
    args: &[String],
    env: &dyn Fn(&str) -> Option<String>,
    read: impl FnOnce(Option<&str>, &str) -> Result<Read, String>,
) -> Result<Option<(i32, String, String)>, String> {
    let [_, _, key] = args else {
        let usage = "usage: see dash help; got: dash review push <owner/repo:branch>\n";
        return Ok(Some((2, String::new(), usage.into())));
    };
    let (repo, rest) = repo_branch(key);
    let (Some((prs, branch)), rows, holds) = read(repo, rest)? else {
        return Ok(None);
    };
    let holders = holders(branch.as_deref(), &prs, &rows, &holds);
    Ok(refusal(key, env, &holders)
        .map(|why| (1, String::new(), format!("review push: refused: {why}\n"))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    fn row(id: &str, status: &str, pane: Option<&str>) -> Standing {
        Standing {
            id: id.into(),
            status: status.into(),
            pane: pane.map(String::from),
            ..Standing::default()
        }
    }

    fn hold(pr: u64, holder: &str) -> Hold {
        Hold {
            pr,
            holder: holder.into(),
            why: "a stand-in".into(),
            touched_ms: 0,
        }
    }

    /// `dash review push <key>` with this environment, over stand-in PRs, rows and holds.
    fn ask(key: &str, env: &[(&str, &str)], read: Read) -> Option<(i32, String, String)> {
        let env = |k: &str| {
            env.iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        };
        let args = ["review", "push", key].map(String::from);
        answer(&args, &env, |_, _| Ok(read)).expect("read")
    }

    /// Done-when 3: a stand-in holder, a Rust run reviewing the branch in its pane, refuses another session's push in
    /// push_refusal's words, and its own task's session is told it is not the pusher; the holder itself, and a branch
    /// nobody in Rust holds, go on to the older app, which judges its own claims.
    #[dash_test]
    fn review_push_refuses_as_the_older_app_did() {
        let key = "nha-org/site_perso:task/x";
        let read = || {
            (
                Some((vec![], Some("task/x".into()))),
                vec![
                    row("review-x", "claimed", Some("w8:pHJ")),
                    row("y", "claimed", None),
                ],
                vec![],
            )
        };
        assert_eq!(
            ask(key, &[("DASH_SESSION", "other")], read()),
            Some((
                1,
                String::new(),
                "review push: refused: review-x (pane w8:pHJ) holds the PR of branch task/x (a live review claim, merge on \
                 green's reviewer, the PM check's reviewer at work, or the owner dash task pr named). Let it finish, or send \
                 it your change (dash pane say w8:pHJ <text>); its own session pushes as it is, named by its DASH_SESSION or \
                 REVIEW_CLAIM_WHO.\n"
                    .into()
            ))
        );
        assert_eq!(
            ask(key, &[("DASH_SESSION", "x")], read()).map(|a| a.2),
            Some(
                "review push: refused: review-x (pane w8:pHJ) holds your PR (branch task/x: its reviewer, or the owner dash \
                 task pr named) and is its one pusher. Push nothing: send it your fix (dash pane say w8:pHJ <text>), or wait \
                 for its end.\n"
                    .into()
            )
        );
        assert_eq!(ask(key, &[("DASH_SESSION", "review-x")], read()), None);
        // the Rust launcher's mark: rust-<id> is <id>, the reviewer itself and the task's own session alike
        assert_eq!(ask(key, &[("DASH_SESSION", "rust-review-x")], read()), None);
        let own = ask(key, &[("DASH_SESSION", "rust-x")], read()).map(|a| a.2);
        assert!(
            own.is_some_and(|said| said.contains("holds your PR")),
            "its own session is told so"
        );
        assert_eq!(ask(key, &[("REVIEW_CLAIM_WHO", "review-x")], read()), None);
        // its stored pane never exempts a pusher: herdr gives a dead run's pane to the next session
        assert!(ask(key, &[("HERDR_PANE_ID", "w8:pHJ")], read()).is_some());
        let (_, rows, holds) = read();
        let z = (Some((vec![], Some("task/z".into()))), rows, holds);
        assert_eq!(ask("nha-org/site_perso:task/z", &[], z), None);
    }

    /// A PR link is held by the claimed reviewer of the branch the mirror names for it, as its branch is (/code-review of
    /// PR 1604: a link skipped the claims, and the review claim hook let another session through); a claim whose run is
    /// gone, read as no longer `claimed`, holds nothing.
    #[dash_test]
    fn a_link_is_held_by_its_branch_s_claimed_reviewer() {
        let link = "https://github.com/nha-org/site_perso/pull/7";
        let read = |status: &str| {
            (
                Some((vec![7], Some("task/x".into()))),
                vec![row("review-x", status, Some("w1:p2"))],
                vec![],
            )
        };
        let said = ask(link, &[("DASH_SESSION", "other")], read("claimed")).expect("refused");
        assert!(
            said.2
                .starts_with("review push: refused: review-x (pane w1:p2) holds the PR"),
            "{}",
            said.2
        );
        assert_eq!(
            ask(link, &[("DASH_SESSION", "rust-review-x")], read("claimed")),
            None
        );
        assert_eq!(
            ask(link, &[("DASH_SESSION", "other")], read("run gone")),
            None
        );
    }

    /// A run's hold on the branch's PR refuses, named with no pane; the lane's and the orchestrator's hold no push.
    #[dash_test]
    fn a_run_s_hold_on_the_branch_s_pr_refuses() {
        let holds = vec![
            hold(7, "run review-pm-7"),
            hold(7, "review-lane"),
            hold(7, "orchestrator"),
            hold(8, "run w"),
        ];
        let said = ask(
            "nha-org/site_perso:task/foo",
            &[],
            (Some((vec![7], Some("task/foo".into()))), vec![], holds),
        )
        .expect("a refusal")
        .2;
        assert!(
            said.starts_with(
                "review push: refused: review-pm-7 (no live pane) holds the PR of branch task/foo"
            ),
            "{said}"
        );
        let lane = vec![hold(7, "review-lane"), hold(7, "orchestrator")];
        assert_eq!(
            ask(
                "nha-org/site_perso:task/foo",
                &[],
                (Some((vec![7], Some("task/foo".into()))), vec![], lane)
            ),
            None
        );
        // the task's own run, claimed and holding its PR through its landing, refuses nobody: its session (rust-foo), its
        // update from main (no name) and a second reviewer all push, by the branch and by the PR's link
        let landing = || {
            (
                Some((vec![7], Some("task/foo".into()))),
                vec![row("foo", "claimed", Some("w1:p1"))],
                vec![hold(7, "run foo")],
            )
        };
        for env in [
            &[][..],
            &[("DASH_SESSION", "rust-foo")],
            &[("REVIEW_CLAIM_WHO", "orch-review-7")],
        ] {
            assert_eq!(ask("nha-org/site_perso:task/foo", env, landing()), None);
            let link = "https://github.com/nha-org/site_perso/pull/7";
            assert_eq!(ask(link, env, landing()), None);
        }
    }

    /// Another repository's key is the older app's to judge, its read naming none; a misuse gets the older CLI's usage; a
    /// key is split as repo_branch/1 splits it.
    #[dash_test]
    fn another_repo_goes_on_and_a_misuse_gets_the_usage() {
        let env = |_: &str| None;
        let args = |w: &[&str]| w.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let theirs = |_: Option<&str>, _: &str| Ok((None, vec![row("x", "claimed", None)], vec![]));
        assert_eq!(
            answer(
                &args(&["review", "push", "nha-org/klar:task/x"]),
                &env,
                theirs
            ),
            Ok(None)
        );
        let unread = |_: Option<&str>, _: &str| Err::<Read, String>("never read".into());
        let (code, _, err) = answer(&args(&["review", "push"]), &env, unread)
            .expect("usage")
            .expect("Rust's");
        assert_eq!(
            (code, err.as_str()),
            (
                2,
                "usage: see dash help; got: dash review push <owner/repo:branch>\n"
            )
        );
        assert_eq!(
            repo_branch("nha-org/site_perso:task/x"),
            (Some("nha-org/site_perso"), "task/x")
        );
        assert_eq!(
            repo_branch("/site_perso:task/x"),
            (None, "/site_perso:task/x")
        );
        let link = "https://github.com/nha-org/site_perso/pull/7";
        assert_eq!(repo_branch(link), (None, link));
    }
}
