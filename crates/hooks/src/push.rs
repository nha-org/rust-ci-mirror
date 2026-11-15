//! `dash review push <owner/repo:branch | PR link>`, the pre-push hook's question, judged first from Rust's own runs and holds
//! (task rust-forwarded-next, part 3; @nha, 2026-10-22: « No double work. »): a push to a PR a Rust run holds is refused,
//! each holder named with its pane, in `Dashboard.Orch.PM.push_refusal/3`'s words; the pusher is named by its environment,
//! as there. Review claims, merge on green's pushers and its `dash task pr` owner are Rust's rows too (`crate::claims`), and the
//! older app's reviewers are read from herdr, so no push asks the older app (task rust-review-claims-port).
//! ponytail: the lane's hold and the orchestrator's hold no push, as the lane's reviewer pushes its fixes; a holder's stored
//! pane names it and never exempts a pusher, since herdr gives a dead run's pane to the next session: a Rust run is
//! exempt by its DASH_SESSION.
//! A task's own run holds no push, as `push_refusal/3` never counted it (second review of PR 1604): its row is claimed and
//! its `run <id>` hold stands for its whole landing, while its own session (`DASH_SESSION=rust-<id>`, the launcher's mark),
//! its run's update from main (no session named) and its reviewers push to its branch. Only a reviewer's run holds one.
use crate::claims::ReviewClaim;
use dash_core::tasks::{Hold, Standing};

/// A session holding the PR: its name (a task id, `review-<id>`) and the herdr pane it works in; `seen`: herdr shows it there
/// now, so a push from that pane is its own.
#[derive(Debug, PartialEq)]
pub struct Holder {
    pub name: String,
    pub pane: Option<String>,
    pub seen: bool,
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

/// The sessions holding the PR: a claimed run of the branch's review (`review-<id>`), a reviewer's run holding one of the PRs
/// the key names, the older app's reviewer of the branch herdr shows at work, and each live claim on one of them, merge on
/// green's named by its pusher. Never the task's own run.
pub fn holders(read: &Read) -> Vec<Holder> {
    let pane = |id: &str| {
        (read.rows.iter())
            .find(|r| r.id == id)
            .and_then(|r| r.pane.clone())
    };
    let claimed = |id: &str| (read.rows.iter()).any(|r| r.id == id && r.status == "claimed");
    let mut out: Vec<Holder> = vec![];
    // `at`: the pane it works in and whether the terminal shows it there now; a holder named twice keeps the pane seen, so
    // its own push from there is never refused (/code-review)
    let mut add = |name: String, at: Option<(&String, bool)>| {
        let (pane, seen) = at.map_or_else(|| (pane(&name), false), |(p, s)| (Some(p.clone()), s));
        match out.iter_mut().find(|h| h.name == name) {
            Some(h) if seen && !h.seen => (h.pane, h.seen) = (pane, seen),
            Some(_) => {}
            None => out.push(Holder { name, pane, seen }),
        }
    };
    if let Some(id) = read.branch.as_deref().and_then(|b| b.strip_prefix("task/")) {
        let name = format!("review-{id}");
        if claimed(&name) {
            add(name.clone(), None);
        }
        if let Some((_, p)) = read.reviewers.iter().find(|(t, _)| t == id) {
            add(name, Some((p, true)));
        }
    }
    // the PM check's reviewer of a PR no task owns, `task-review-pr-<repo>-<n>`, holds that PR (`PM.holding_reviewer/2` by
    // link; /code-review of PR 1651)
    for (id, p) in &read.reviewers {
        let n = id.strip_prefix(&format!("pr-{}-", read.repo));
        if n.and_then(|n| n.parse().ok())
            .is_some_and(|n| read.prs.contains(&n))
        {
            add(format!("review-{id}"), Some((p, true)));
        }
    }
    for h in read.holds.iter().filter(|h| read.prs.contains(&h.pr)) {
        let run = h.holder.strip_prefix("run ");
        if let Some(id) = run.filter(|id| id.starts_with("review-")) {
            add(id.to_string(), None);
        }
    }
    let on =
        |c: &&ReviewClaim| (read.prs.iter()).any(|n| crate::claims::hits(&c.pr, &read.repo, *n));
    for c in read.claims.iter().filter(on) {
        // a claim's pane vouches for its pusher only while the terminal named its session there (`claims::live`)
        let at = c.pane.as_ref().map(|p| (p, c.session.is_some()));
        add(crate::claims::pusher(c).to_string(), at);
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
    let here = set("HERDR_PANE_ID");
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
        .filter(|h| !(h.seen && h.pane.is_some() && h.pane == here))
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

/// What `answer` reads: the key's repository by name (« site_perso »), the open PRs it names and its branch (a link's as the
/// PR mirror names it); Rust's rows, a claim whose run is gone no longer `claimed`, and live holds (site_perso's alone); the
/// live review claims (`crate::claims::live`); the older app's reviewers herdr shows at work, (task id, pane).
#[derive(Debug, Default)]
pub struct Read {
    pub repo: String,
    pub prs: Vec<u64>,
    pub branch: Option<String>,
    pub rows: Vec<Standing>,
    pub holds: Vec<Hold>,
    pub claims: Vec<ReviewClaim>,
    pub reviewers: Vec<(String, String)>,
}

/// `Read` from the host's task store under `dir` (site_perso's alone) and this plugin's claims: `named`, the key's repository,
/// PRs and branch as the PR mirror names them; `reviewers`, the older app's reviewers the terminal shows (repository, task id,
/// pane), the key's repository's alone kept (/code-review: a site_perso reviewer held klar's branch of the same name).
pub fn read(
    host: &dyn dash_core::Host,
    dir: &std::path::Path,
    named: (String, Vec<u64>, Option<String>),
    reviewers: Reviewers,
) -> Result<Read, String> {
    let (repo, prs, branch) = named;
    let reviewers = (reviewers.into_iter())
        .filter(|(r, _, _)| r.eq_ignore_ascii_case(&repo))
        .map(|(_, id, pane)| (id, pane))
        .collect();
    // the host's ports opened once, for the claims' panes and site_perso's rows (/code-review of PR 1651)
    let ports = host.tasks(dir)?;
    let on = |c: &ReviewClaim| prs.iter().any(|n| crate::claims::hits(&c.pr, &repo, *n));
    let claims = crate::claims::now(host, dir, Some(&ports), &on)?;
    let store = &ports.store;
    let (mut rows, holds, gone) = match repo == "site_perso" {
        // a lock this read cannot look at says nothing, as `run_gone`: its claim still holds (/code-review of PR 1651)
        true => (
            store.standing()?,
            store.holds()?,
            store.crashed().unwrap_or_default(),
        ),
        false => (vec![], vec![], vec![]),
    };
    // a claim whose run is gone holds nothing: its row stays claimed until the next look (/code-review of PR 1604)
    for r in rows.iter_mut().filter(|r| r.status == "claimed") {
        if gone.iter().any(|c| c.id == r.id) {
            r.status = "run gone".into();
        }
    }
    Ok(Read {
        repo,
        prs,
        branch,
        rows,
        holds,
        claims,
        reviewers,
    })
}

/// `read` with herdr's answer `seen`: unread is an error, never « no reviewer at work », so the push goes to the older app
/// (/code-review of PR 1651), unless what was read holds the PR already, which judges it with no older app (/code-review
/// medium of PR 1651).
pub fn read_seen(
    host: &dyn dash_core::Host,
    dir: &std::path::Path,
    named: (String, Vec<u64>, Option<String>),
    seen: Result<Reviewers, String>,
) -> Result<Read, String> {
    match seen {
        Ok(seen) => read(host, dir, named, seen),
        Err(e) => {
            let read = read(host, dir, named, vec![])?;
            (!holders(&read).is_empty()).then_some(read).ok_or(e)
        }
    }
}

/// The older app's reviewers the terminal shows: (repository, task id, pane).
pub type Reviewers = Vec<(String, String, String)>;

/// Rust alone's claim or release (`crate::claims::alone`), a claim refused past the stored ones by `held_or_shut`, with the
/// reviewers herdr shows (`seen`): the binary's one call.
pub fn claim_alone(
    host: &dyn dash_core::Host,
    dir: &std::path::Path,
    args: &[String],
    env: &dyn Fn(&str) -> Option<String>,
    seen: fn() -> Result<Reviewers, String>,
) -> Option<(i32, String, String)> {
    // herdr unread names no reviewer: the stored claims and the holds still judge
    let held = |pr: &str, who: &str| held_or_shut(host, dir, pr, who, seen().unwrap_or_default());
    crate::claims::alone(host, dir, args, env, &held)
}

/// Why Rust alone refuses `who`'s claim on `pr` past the stored claims, as `PM.claim_review_now`: a link not open, or a PR
/// another session holds (`holders`: the PM check's reviewer at work, a review run's hold); a forge or a store
/// that does not answer refuses nothing (/code-review medium of PR 1651). A Rust-owned PR's refusal is not kept: with the
/// older app gone, Rust's merge is the only one.
pub fn held_or_shut(
    host: &dyn dash_core::Host,
    dir: &std::path::Path,
    pr: &str,
    who: &str,
    reviewers: Reviewers,
) -> Option<String> {
    for l in crate::claims::links(pr) {
        let (state, task) = crate::claims::forge_of(host, &l).unwrap_or(("OPEN".into(), None));
        if state != "OPEN" {
            return Some(format!("review claim: {l} is not open"));
        }
        let (repo, n, _) = dash_core::pr_key(&l)?;
        let branch = task.map(|t| format!("task/{t}"));
        let read = read(host, dir, (repo, vec![n], branch), reviewers.clone()).ok()?;
        let others: Vec<String> = (holders(&read).into_iter())
            .map(|h| h.name)
            .filter(|h| h != who)
            .collect();
        if !others.is_empty() {
            return Some(format!(
                "review claim: {} reviews {l} already: resume it with dash task review",
                others.join(", ")
            ));
        }
    }
    None
}

/// `dash review push` as `Dashboard.CLI` answered it, `(exit, stdout, stderr)`: a refusal, a misuse, or exit 0 with nothing
/// said. `read` takes the key's repository and the rest; None for a key naming no repository, which nothing holds.
pub fn answer(
    args: &[String],
    env: &dyn Fn(&str) -> Option<String>,
    read: impl FnOnce(Option<&str>, &str) -> Result<Option<Read>, String>,
) -> Result<(i32, String, String), String> {
    let [_, _, key] = args else {
        let usage = "usage: see dash help; got: dash review push <owner/repo:branch>\n";
        return Ok((2, String::new(), usage.into()));
    };
    let (repo, rest) = repo_branch(key);
    let Some(read) = read(repo, rest)? else {
        return Ok((0, String::new(), String::new()));
    };
    Ok(match refusal(key, env, &holders(&read)) {
        Some(why) => (1, String::new(), format!("review push: refused: {why}\n")),
        None => (0, String::new(), String::new()),
    })
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

    /// The PRs and branch a key names (None: no repository), Rust's rows and holds.
    type Old = (Option<(Vec<u64>, Option<String>)>, Vec<Standing>, Vec<Hold>);

    /// `dash review push <key>` with this environment, over stand-in PRs, rows and holds of site_perso: None when it may push.
    fn ask(key: &str, env: &[(&str, &str)], read: Old) -> Option<(i32, String, String)> {
        let (named, rows, holds) = read;
        let read = named.map(|(prs, branch)| Read {
            repo: "site_perso".into(),
            prs,
            branch,
            rows,
            holds,
            ..Read::default()
        });
        asked(key, env, read)
    }

    fn asked(key: &str, env: &[(&str, &str)], read: Option<Read>) -> Option<(i32, String, String)> {
        let env = |k: &str| {
            env.iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        };
        let args = ["review", "push", key].map(String::from);
        let said = answer(&args, &env, |_, _| Ok(read)).expect("read");
        (said != (0, String::new(), String::new())).then_some(said)
    }

    /// Done-when 1 (task rust-review-claims-port): a stand-in claim refuses another session's push to its PR, by its branch
    /// and by its link, with no older app asked; its own session (by name, or from the pane herdr shows it in) pushes; merge on
    /// green's claim stands for its reviewer; the older app's reviewer at work in herdr holds its branch.
    #[dash_test]
    fn a_stand_in_claim_refuses_another_session_s_push() {
        let claim = |who: &str, pusher: Option<&str>| ReviewClaim {
            who: who.into(),
            pr: "https://github.com/nha-org/site_perso/pull/7".into(),
            pane: Some("w2:p3".into()),
            session: Some("s1".into()),
            at_ms: 0,
            pusher: pusher.map(String::from),
        };
        let read = |claims: Vec<ReviewClaim>, reviewers: Vec<(String, String)>| {
            Some(Read {
                repo: "site_perso".into(),
                prs: vec![7],
                branch: Some("task/foo".into()),
                claims,
                reviewers,
                ..Read::default()
            })
        };
        let key = "nha-org/site_perso:task/foo";
        let said = asked(
            key,
            &[("DASH_SESSION", "other")],
            read(vec![claim("orch-review-7", None)], vec![]),
        );
        assert_eq!(
            said.expect("refused").2,
            "review push: refused: orch-review-7 (pane w2:p3) holds the PR of branch task/foo (a live review claim, merge on \
             green's reviewer, the PM check's reviewer at work, or the owner dash task pr named). Let it finish, or send it your \
             change (dash pane say w2:p3 <text>); its own session pushes as it is, named by its DASH_SESSION or REVIEW_CLAIM_WHO.\n"
        );
        let link = "https://github.com/nha-org/site_perso/pull/7";
        assert!(asked(link, &[], read(vec![claim("orch-review-7", None)], vec![])).is_some());
        let own = [("REVIEW_CLAIM_WHO", "orch-review-7")];
        assert_eq!(
            asked(key, &own, read(vec![claim("orch-review-7", None)], vec![])),
            None
        );
        let from_its_pane = [("HERDR_PANE_ID", "w2:p3")];
        assert_eq!(
            asked(
                key,
                &from_its_pane,
                read(vec![claim("orch-review-7", None)], vec![])
            ),
            None
        );
        let handed = read(
            vec![claim(crate::claims::MERGE_ON_GREEN, Some("review-foo"))],
            vec![],
        );
        let said = asked(key, &[("DASH_SESSION", "foo")], handed)
            .expect("refused")
            .2;
        assert!(
            said.contains("review-foo (pane w2:p3) holds your PR"),
            "{said}"
        );
        let theirs = read(vec![], vec![("foo".into(), "w9:p1".into())]);
        let said = asked(key, &[("DASH_SESSION", "other")], theirs)
            .expect("refused")
            .2;
        assert!(
            said.contains("review-foo (pane w9:p1) holds the PR"),
            "{said}"
        );
        // /code-review of 858f82e55: a claim the terminal could not vouch for (its session unread) never exempts a push from
        // its stored pane; a claimed review row beside the reviewer herdr shows keeps the pane seen, so its own push goes
        let unread = ReviewClaim {
            session: None,
            ..claim("orch-review-7", None)
        };
        assert!(asked(key, &from_its_pane, read(vec![unread], vec![])).is_some());
        let both = Read {
            rows: vec![row("review-foo", "claimed", None)],
            ..read(vec![], vec![("foo".into(), "w9:p1".into())]).expect("read")
        };
        assert_eq!(asked(key, &[("HERDR_PANE_ID", "w9:p1")], Some(both)), None);
        // /code-review of PR 1651: the PM check's reviewer of a PR no task owns holds that PR by its number, another's none
        let by_pr = |id: &str| read(vec![], vec![(id.into(), "w8:p1".into())]);
        let said = asked(key, &[], by_pr("pr-site_perso-7"))
            .expect("refused")
            .2;
        assert!(
            said.contains("review-pr-site_perso-7 (pane w8:p1)"),
            "{said}"
        );
        assert_eq!(asked(key, &[], by_pr("pr-site_perso-8")), None);
        assert_eq!(asked(key, &[], by_pr("pr-klar-7")), None);
        // another PR's claim, and a key naming no repository, hold nothing
        let elsewhere = ReviewClaim {
            pr: "https://github.com/nha-org/klar/pull/7".into(),
            ..claim("x", None)
        };
        assert_eq!(asked(key, &[], read(vec![elsewhere], vec![])), None);
        assert_eq!(asked("nowhere", &[], None), None);
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

    /// A key whose read names no PR pushes, with no older app asked; a misuse gets the older CLI's usage; a
    /// key is split as repo_branch/1 splits it.
    #[dash_test]
    fn another_repo_goes_on_and_a_misuse_gets_the_usage() {
        let env = |_: &str| None;
        let args = |w: &[&str]| w.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let theirs = |_: Option<&str>, _: &str| Ok(None);
        assert_eq!(
            answer(
                &args(&["review", "push", "nha-org/klar:task/x"]),
                &env,
                theirs
            ),
            Ok((0, String::new(), String::new()))
        );
        let unread = |_: Option<&str>, _: &str| Err::<Option<Read>, String>("never read".into());
        let (code, _, err) = answer(&args(&["review", "push"]), &env, unread).expect("usage");
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
