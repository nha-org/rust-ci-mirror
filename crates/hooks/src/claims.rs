//! Review claims in Rust's store (task rust-review-claims-port; @nha, 2026-11-14: « If you don't need it killing elixir should
//! reclaim ram/cpu too. »): `dash review claim`, `review release`, merge on green's hand-over (`review handover`) and its new
//! owner (`task pr`) each write a row per PR link, which `dash review push` and `dash review next` read with no older app.
//! While the older app answers, it judges each of these and Rust records what it took, so its PM check still passes a claimed
//! PR over; once it is gone, Rust judges a claim and a release alone.
//! ponytail: a lapse is read again at each look, never kept as the older app's memory did: a session herdr misses once holds
//! nothing for that look, and holds again once seen. Rust alone ends a release without asking whether the claim's pane runs a
//! turn, which the older app refused; a busy-pane check from herdr's status if a release ever orphans a review.
use dash_core::{FieldWrite, Host, LANE, LANE_HOLD_MS, OpLog, TaskPorts, Value};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

/// A review claim on one PR link (task rust-review-claims-port): `who` holds `pr` since `at_ms`, from the terminal's pane
/// `pane` while its session `session` stays there; merge on green's claim (`MERGE_ON_GREEN`) names the session that pushes
/// for it, `pusher`. It lapses after `LANE_HOLD_MS`, the older app's 6 hours.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReviewClaim {
    pub who: String,
    pub pr: String,
    pub pane: Option<String>,
    pub session: Option<String>,
    pub at_ms: i64,
    pub pusher: Option<String>,
}

/// The older app's merge on green, as a claim's holder: `dash review handover` hands it a PR.
pub const MERGE_ON_GREEN: &str = "merge-on-green";

/// This plugin's database, under the host's data folder: one row per holder and link, `id` « <who> <link> ».
pub const FILE: &str = "claims.sqlite";
pub const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS claims (id TEXT PRIMARY KEY, who TEXT, pr TEXT, pane TEXT, session TEXT,
  pusher TEXT) WITHOUT ROWID;";

/// Each claim written within `LANE_HOLD_MS` of `now_ms` and not ended, oldest first, its time its last write's: the log's
/// recent ops replayed a page at a time, since an older claim holds nothing (rust/CLAUDE.md point 5: never the whole table).
pub fn stored(log: &dyn OpLog, now_ms: i64) -> Result<Vec<ReviewClaim>, String> {
    let mut rows: BTreeMap<String, ReviewClaim> = BTreeMap::new();
    let mut after = ((now_ms - LANE_HOLD_MS).max(0) << 16, String::new());
    loop {
        let ops = log.ops_after(Some((after.0, &after.1)), 500)?;
        let Some(last) = ops.last() else { break };
        after = (last.stamp, last.writer.clone());
        for op in ops.iter().filter(|o| o.write.table == "claims") {
            let text = match &op.write.value {
                Value::Text(t) => Some(t.clone()),
                _ => None,
            };
            if op.write.field == "who" && text.is_none() {
                rows.remove(&op.write.row);
                continue;
            }
            let row = rows.entry(op.write.row.clone()).or_default();
            match op.write.field.as_str() {
                "who" => (row.who, row.at_ms) = (text.unwrap_or_default(), op.stamp >> 16),
                "pr" => row.pr = text.unwrap_or_default(),
                "pane" => row.pane = text,
                "session" => row.session = text,
                "pusher" => row.pusher = text,
                _ => {}
            }
        }
    }
    let mut all: Vec<ReviewClaim> = (rows.into_values()).filter(|c| !c.who.is_empty()).collect();
    all.sort_by_key(|c| c.at_ms);
    Ok(all)
}

/// `change` written to `log` in one commit: each claim's row in full, its holder last, each end its holder cleared.
pub fn write(log: &mut dyn OpLog, change: &Change) -> Result<(), String> {
    let field = |row: &str, name: &str, value: Option<&String>| FieldWrite {
        table: "claims".into(),
        row: row.into(),
        field: name.into(),
        value: value.map_or(Value::Null, |v| Value::Text(v.clone())),
    };
    let mut writes = vec![];
    for c in &change.claims {
        let row = format!("{} {}", c.who, c.pr);
        let fields = [
            ("pr", Some(&c.pr)),
            ("pane", c.pane.as_ref()),
            ("session", c.session.as_ref()),
            ("pusher", c.pusher.as_ref()),
            ("who", Some(&c.who)),
        ];
        writes.extend(fields.map(|(n, v)| field(&row, n, v)));
    }
    for (who, pr) in &change.ends {
        writes.push(field(&format!("{who} {pr}"), "who", None));
    }
    match writes.is_empty() {
        true => Ok(()),
        false => log.write(&writes).map(drop),
    }
}

/// The session the host's terminal (`ports`) shows in pane `p` (`live`'s `seat_of`): None when it is unread, so the claim
/// lapses nothing.
pub fn seat(ports: Option<&TaskPorts>, p: &str) -> Option<Option<String>> {
    Some(ports?.terminal.agent(p).ok()?.and_then(|s| s.session))
}

/// The live claims `keep` takes in the host's database under `dir`, each claim's pane asked of the host's terminal (`seat`)
/// once kept (/simplify of PR 1651: a push asked herdr for every PR's claims), from `ports` the caller opened already
/// (`crate::push::read`), else opened here only when a kept claim has a pane.
pub fn now(
    host: &dyn Host,
    dir: &Path,
    ports: Option<&TaskPorts>,
    keep: &dyn Fn(&ReviewClaim) -> bool,
) -> Result<Vec<ReviewClaim>, String> {
    let all = stored(&*host.open(dir, FILE, &[SCHEMA])?, host.now_ms())?;
    let kept: Vec<ReviewClaim> = all.into_iter().filter(|c| keep(c)).collect();
    let opened = (ports.is_none() && kept.iter().any(|c| c.pane.is_some()))
        .then(|| host.tasks(dir).ok())
        .flatten();
    let ports = ports.or(opened.as_ref());
    Ok(live(kept, &|p| seat(ports, p)))
}

/// Why a claim's PR (`pr`, `who`) is held or shut past the stored claims: `crate::push::held_or_shut`.
pub type Held<'a> = &'a dyn Fn(&str, &str) -> Option<String>;

/// bin/dash's `args` with no older app to judge them, `env` its environment: Rust's answer, `(exit, stdout, stderr)`; None for
/// a command Rust alone does not judge. `held`: why a claim's PR is held or shut past the stored claims
/// (`crate::push::held_or_shut`), given by the caller so this module never reads the push's holders.
pub fn alone(
    host: &dyn Host,
    dir: &Path,
    args: &[String],
    env: &dyn Fn(&str) -> Option<String>,
    held: Held,
) -> Option<(i32, String, String)> {
    let said = command(
        host,
        dir,
        &Asked::of(args, &|| home(env))?,
        pane(env),
        Some(held),
    )?;
    Some(said.map_or_else(|e| (1, String::new(), e + "\n"), |s| (0, s, String::new())))
}

/// What Rust adds after the older app answered `args` with `exit`: what it took (exit 0) written here too, a write that failed
/// said; nothing else.
pub fn after(
    host: &dyn Host,
    dir: &Path,
    args: &[String],
    env: &dyn Fn(&str) -> Option<String>,
    exit: Option<i32>,
) -> String {
    let Some(asked) = Asked::of(args, &|| home(env)).filter(|_| exit == Some(0)) else {
        return String::new();
    };
    match command(host, dir, &asked, pane(env), None) {
        Some(Err(e)) => format!("Rust's claims kept nothing: {e}\n"),
        _ => String::new(),
    }
}

/// A PR link's state (« OPEN ») and task, from its branch (`task/<id>`) as the forge names them: the reviewer merge on green
/// resumes for a helper's hand-over (`Handover.reviewer/3`; /code-review of PR 1651), and whether Rust alone takes a claim.
pub fn forge_of(host: &dyn Host, link: &str) -> Option<(String, Option<String>)> {
    let q = r#".state + " " + .headRefName"#;
    let said = host
        .forge(&["pr", "view", link, "--json", "state,headRefName", "-q", q])
        .ok()?;
    let (state, branch) = said.trim().split_once(' ')?;
    Some((
        state.to_string(),
        branch.strip_prefix("task/").map(String::from),
    ))
}

/// The caller's checkout's « owner/repo », from its PWD's origin: whose PR a bare number names, as the older app's
/// `number_link/2` reads it (/code-review of PR 1651).
fn home(env: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let cwd = env("PWD")?;
    let origin = ["remote", "get-url", "origin"];
    let url = dash_process::run(
        "git",
        &origin,
        Some(Path::new(&cwd)),
        Duration::from_secs(5),
    );
    dash_core::github_repo(url.ok()?.trim())
}

/// The claimer's herdr pane, from its environment.
fn pane(env: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    env("HERDR_PANE_ID").filter(|p| !p.is_empty())
}

/// Rust alone's claims judged one at a time: of two claims at once, one (the older app's `one_start`; /code-review medium of PR
/// 1651). ponytail: one process lock, since every claim reaches the front door; a store lock if another process ever judges.
static ONE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// `asked` run on the host's database under `dir`, from the claimer's herdr `pane` (HERDR_PANE_ID): what the older app took
/// (`alone` None: Ok with nothing to say, or the write's failure), or judged by Rust alone (`alone`, why a claim's PR is held
/// or shut past the stored claims: `crate::push::held_or_shut`), its words or its refusal. None: a command Rust alone does not judge.
fn command(
    host: &dyn Host,
    dir: &Path,
    asked: &Asked,
    pane: Option<String>,
    alone: Option<Held>,
) -> Option<Result<String, String>> {
    // a hand-over or a new owner without the older app is its merge on green's: nothing read for it (/code-review)
    if alone.is_some() && matches!(asked, Asked::Handover { .. } | Asked::Repoint { .. }) {
        return None;
    }
    // the forge and herdr asked before the lock, so a slow gh never holds another claim back (/simplify of PR 1651)
    if let (Some(held_or_shut), Asked::Claim { pr, who }) = (alone, asked)
        && let Some(why) = held_or_shut(pr, who)
    {
        return Some(Err(why));
    }
    let _one = alone.map(|_| ONE.lock().unwrap_or_else(|e| e.into_inner()));
    let read = host
        .open(dir, FILE, &[SCHEMA])
        .and_then(|log| Ok((stored(&*log, host.now_ms())?, log)));
    let (all, mut log) = match read {
        Ok(read) => read,
        Err(e) => return Some(Err(e)),
    };
    // the terminal only when a pane is read: a release reads none (/simplify of PR 1651)
    let release = matches!(asked, Asked::Release { .. });
    let panes = pane.is_some() || (!release && all.iter().any(|c| c.pane.is_some()));
    let ports = panes.then(|| host.tasks(dir).ok()).flatten();
    let seat_of = |p: &str| seat(ports.as_ref(), p);
    // a pane whose session the terminal does not name is no proof of the claimer: the 6 hours then, as the older app
    let seat = pane.and_then(|p| Some((p.clone(), seat_of(&p)??)));
    // a release ends every stored claim it names, its session seen or not: one herdr missed once would hold again (/code-review)
    let basis = match release {
        true => all,
        false => live(all, &seat_of),
    };
    let (changed, said) = match alone {
        None => (
            change(asked, seat, &basis, &|l| forge_of(host, l)?.1),
            String::new(),
        ),
        Some(_) => match judge(asked, seat, &basis)? {
            Ok(judged) => judged,
            Err(refused) => return Some(Err(refused)),
        },
    };
    Some(write(&mut *log, &changed).map(|_| said))
}

/// The command a claim row moves with: what `of` reads in bin/dash's words.
#[derive(Debug, PartialEq)]
pub enum Asked {
    Claim { pr: String, who: String },
    Release { target: String, all: bool },
    Handover { pr: String, reviewer: String },
    Repoint { pr: String, task: String },
}

impl Asked {
    /// `home`: the caller's « owner/repo », whose PR a bare number or « #n » names for a hand-over and `task pr`, as the
    /// older app's `number_link/2` reads it from the cwd's origin (/code-review of PR 1651), asked only then; site_perso's
    /// with none.
    pub fn of(args: &[String], home: &dyn Fn() -> Option<String>) -> Option<Asked> {
        let words: Vec<&str> = args.iter().map(String::as_str).collect();
        let s = |w: &str| w.to_string();
        let link = |w: &str| match number(w) {
            Some(n) => format!(
                "https://github.com/{}/pull/{n}",
                home().as_deref().unwrap_or("nha-org/site_perso")
            ),
            None => w.to_string(),
        };
        Some(match words[..] {
            ["review", "claim", pr, who] => Asked::Claim {
                pr: s(pr),
                who: s(who),
            },
            ["review", "release", target] => Asked::Release {
                target: s(target),
                all: false,
            },
            ["review", "release", target, "--all"] => Asked::Release {
                target: s(target),
                all: true,
            },
            ["review", "handover", pr, reviewer] => Asked::Handover {
                pr: link(pr),
                reviewer: s(reviewer),
            },
            ["task", "pr", pr, task] if pr.contains("/pull/") || number(pr).is_some() => {
                Asked::Repoint {
                    pr: link(pr),
                    task: s(task),
                }
            }
            _ => return None,
        })
    }
}

/// A PR link's repository by name, lowercase, and its number, whatever follows it (« /files », « #issuecomment-1 »).
fn key(link: &str) -> Option<(String, u64)> {
    dash_core::pr_key(link).map(|(repo, n, _)| (repo, n))
}

/// A bare PR number, « 7 » or « #7 ».
fn number(w: &str) -> Option<u64> {
    let n = w.trim().trim_start_matches('#');
    n.chars()
        .all(|c| c.is_ascii_digit())
        .then(|| n.parse().ok())?
}

/// The PR links in `pr`, each cut to `https://github.com/<owner>/<repo>/pull/<n>`: its http words alone, as `PM.links/1` (a
/// claim by bare number holds no link; `Asked::of` resolves a hand-over's).
pub fn links(pr: &str) -> Vec<String> {
    (pr.split_whitespace())
        .filter(|w| w.starts_with("http"))
        .filter_map(|w| Some(dash_core::pr_key(w)?.2.to_string()))
        .collect()
}

/// Whether two links name the same PR: its repository by name and its number, so an owner from before the move to nha-org or
/// another case is the same.
pub fn same(a: &str, b: &str) -> bool {
    key(a).is_some_and(|k| key(b) == Some(k))
}

/// Whether `link` is PR `n` of the repository named `repo` (« site_perso »).
pub fn hits(link: &str, repo: &str, n: u64) -> bool {
    key(link) == Some((repo.to_ascii_lowercase(), n))
}

/// The claims whose session still works where it claimed: one with no pane holds its 6 hours; one with a pane holds while
/// `seat_of` names its session there. A terminal unread (None) lapses nothing, but leaves the claim's session unread: its pane
/// then names it, never vouches for a pusher (`crate::push`).
pub fn live(
    claims: Vec<ReviewClaim>,
    seat_of: &dyn Fn(&str) -> Option<Option<String>>,
) -> Vec<ReviewClaim> {
    let seen = |c: ReviewClaim| match c.pane.as_deref().map(seat_of) {
        None => Some(c),
        Some(None) => Some(ReviewClaim { session: None, ..c }),
        Some(Some(s)) => (s == c.session).then_some(c),
    };
    claims.into_iter().filter_map(seen).collect()
}

/// The site_perso PRs these claims hold, the lane's own left out: what `dash review next` passes over.
pub fn held_prs(live: &[ReviewClaim]) -> Vec<u64> {
    let site = |c: &ReviewClaim| {
        key(&c.pr)
            .filter(|(r, _)| r == "site_perso")
            .map(|(_, n)| n)
    };
    live.iter()
        .filter(|c| c.who != LANE)
        .filter_map(site)
        .collect()
}

/// Who holds a claim's push: merge on green stands for the reviewer it records, else for nobody but itself.
pub fn pusher(c: &ReviewClaim) -> &str {
    match (c.who.as_str(), &c.pusher) {
        (MERGE_ON_GREEN, Some(p)) => p,
        (who, _) => who,
    }
}

/// What a command changes in the store: claims written, claims ended (holder, link).
#[derive(Debug, Default, PartialEq)]
pub struct Change {
    pub claims: Vec<ReviewClaim>,
    pub ends: Vec<(String, String)>,
}

/// A row merge on green or a new owner holds: no pane, so its 6 hours alone end it.
fn seatless(who: &str, pr: &str, pusher: Option<String>) -> ReviewClaim {
    ReviewClaim {
        who: who.into(),
        pr: pr.into(),
        pusher,
        ..ReviewClaim::default()
    }
}

/// What `asked` writes once the older app took it, among the `live` claims: the claimer's pane and its session `seat`;
/// `task_of`, a PR link's task.
pub fn change(
    asked: &Asked,
    seat: Option<(String, String)>,
    live: &[ReviewClaim],
    task_of: &dyn Fn(&str) -> Option<String>,
) -> Change {
    let (pane, session) = seat.map_or((None, None), |(p, s)| (Some(p), Some(s)));
    let claim = |who: &str, pr: String, pusher: Option<String>| ReviewClaim {
        who: who.into(),
        pr,
        pane: pane.clone(),
        session: session.clone(),
        at_ms: 0,
        pusher,
    };
    match asked {
        Asked::Claim { pr, who } => Change {
            claims: links(pr).into_iter().map(|l| claim(who, l, None)).collect(),
            ..Change::default()
        },
        // the older app's hand-over ends the hander's own claim, and resumes on red, so lets push, `Handover.reviewer/3`: a
        // PM reviewer itself, else the PR's task's (/code-review of PR 1651)
        Asked::Handover { pr, reviewer } => {
            let urls = links(pr);
            let resumed = |l: &str| match reviewer.starts_with("review-") && reviewer != LANE {
                true => Some(reviewer.clone()),
                false => task_of(l)
                    .map(|t| format!("review-{t}"))
                    .or(Some(reviewer.clone())),
            };
            // a second hand-over of a PR merge on green holds already changes nothing: its 6 hours never start again (/code-review
            // medium of PR 1651)
            let held = |l: &String| {
                live.iter()
                    .any(|c| c.who == MERGE_ON_GREEN && same(l, &c.pr))
            };
            Change {
                claims: (urls.iter().filter(|l| !held(l)))
                    .map(|l| seatless(MERGE_ON_GREEN, l, resumed(l)))
                    .collect(),
                // each claim of the hander's on that PR, under the link it was stored by
                ends: (live.iter())
                    .filter(|c| c.who == *reviewer && urls.iter().any(|u| same(u, &c.pr)))
                    .map(|c| (c.who.clone(), c.pr.clone()))
                    .collect(),
            }
        }
        // the new owner pushes alone: merge on green's row names it its pusher, else the owner holds a claim of its own, as the
        // older app's `Claims.hand_over` row (/code-review medium of PR 1651)
        Asked::Repoint { pr, task } => {
            let owned = |u: &String| {
                let handed = live
                    .iter()
                    .find(|c| c.who == MERGE_ON_GREEN && same(u, &c.pr));
                handed.map_or_else(
                    || seatless(task, u, None),
                    |c| ReviewClaim {
                        pusher: Some(task.clone()),
                        ..c.clone()
                    },
                )
            };
            Change {
                claims: links(pr).iter().map(owned).collect(),
                ..Change::default()
            }
        }
        Asked::Release { target, all } => {
            // a bare number names that number's PR in every repository, as `PM.claims_on/2`: Rust mirrors what the older app
            // ended (/code-review of PR 1651)
            let urls = links(target.trim());
            let n = number(target);
            let hit = |c: &&ReviewClaim| {
                c.who == target.trim()
                    || urls.iter().any(|u| same(u, &c.pr))
                    || n.is_some_and(|n| key(&c.pr).is_some_and(|(_, m)| m == n))
            };
            // merge on green's claim stays unless --all: a release on 949 ended the app's merge (release-keeps-merge)
            let ends = (live.iter().filter(hit))
                .filter(|c| *all || c.who != MERGE_ON_GREEN)
                .map(|c| (c.who.clone(), c.pr.clone()));
            Change {
                ends: ends.collect(),
                ..Change::default()
            }
        }
    }
}

/// `asked` judged by Rust alone, the older app gone: the change and its words, or the refusal in `Orch.CLI`'s words. A
/// hand-over or a new owner is merge on green's, the older app's: None.
pub fn judge(
    asked: &Asked,
    seat: Option<(String, String)>,
    live: &[ReviewClaim],
) -> Option<Result<(Change, String), String>> {
    let pane = seat.as_ref().map(|(p, _)| p.clone());
    let changed = change(asked, seat, live, &|_| None);
    Some(match asked {
        Asked::Claim { pr, who } => {
            let urls = links(pr);
            let on = |c: &&ReviewClaim| urls.iter().any(|u| same(u, &c.pr));
            let mut held: Vec<&str> = live.iter().filter(on).map(|c| c.who.as_str()).collect();
            held.dedup();
            if urls.is_empty() {
                Err(format!("review claim: {pr} holds no PR link (https://github.com/<owner>/<repo>/pull/<n>)"))
            } else if held.contains(&MERGE_ON_GREEN) {
                Err(format!("review claim: the app holds {pr}'s merge on green CI (dash review handover)"))
            } else if who.starts_with("review-") && who != LANE {
                Err(format!("review claim: {who} is a task or reviewer of the PM check: name the helper (orchestrator-helper)"))
            } else if let others @ [_, ..] = &held.into_iter().filter(|h| h != who).collect::<Vec<_>>()[..] {
                Err(format!("review claim: {} holds a live claim on {pr}: dash review release {pr} ends it", others.join(", ")))
            } else {
                let until = pane.map_or("for 6 hours".into(), |p| format!("until its session in pane {p} ends, 6 hours at most"));
                Ok((changed, format!("review claim: {who} reviews {pr}; the PM check starts no reviewer on it {until}\n")))
            }
        }
        Asked::Release { target, .. } if target == MERGE_ON_GREEN => Err(
            "review release: merge-on-green holds every PR handed to the app; name the PR: dash review release <pr>".into(),
        ),
        Asked::Release { target, .. } => {
            let ended: Vec<String> = changed.ends.iter().map(|(w, l)| format!("{w} on {l}")).collect();
            let said = match ended.is_empty() {
                true => format!("review release: no live claim on {target}\n"),
                false => format!("review release: ended {}\n", ended.join(", ")),
            };
            Ok((changed, said))
        }
        Asked::Handover { .. } | Asked::Repoint { .. } => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    const PR: &str = "https://github.com/nha-org/site_perso/pull/7";

    fn args(words: &str) -> Vec<String> {
        words.split(' ').map(String::from).collect()
    }

    fn claimed(who: &str, pane: Option<&str>) -> ReviewClaim {
        ReviewClaim {
            who: who.into(),
            pr: PR.into(),
            pane: pane.map(String::from),
            session: pane.map(|_| "s1".into()),
            ..ReviewClaim::default()
        }
    }

    /// A claim's session gone from its pane lapses it, another session in that pane too; herdr unread lapses nothing.
    #[dash_test]
    fn a_claim_holds_while_its_session_works_in_its_pane() {
        let all = || vec![claimed("helper", Some("w1:p1")), claimed("orch", None)];
        let seat = |session: &'static str| move |_: &str| Some(Some(session.to_string()));
        assert_eq!(live(all(), &seat("s1")), all());
        let left = live(all(), &seat("s2"));
        assert_eq!(
            left.iter().map(|c| c.who.as_str()).collect::<Vec<_>>(),
            ["orch"]
        );
        // the terminal unread: kept, its session unread, so its pane vouches for no pusher (/code-review)
        let unread = live(all(), &|_| None);
        assert_eq!(unread.len(), 2);
        assert_eq!(
            (unread[0].pane.as_deref(), unread[0].session.as_deref()),
            (Some("w1:p1"), None)
        );
    }

    /// /code-review of 858f82e55: a link is one PR whatever follows its number or names its owner; a bare number released is
    /// that number's PR in every repository, an old owner's link included, as the older app ends it; a claim takes links alone.
    #[dash_test]
    fn a_link_names_one_pr_and_a_release_by_number_ends_every_repository_s() {
        assert_eq!(
            links("https://github.com/nha-org/site_perso/pull/7/files"),
            [PR]
        );
        assert_eq!(
            links("https://github.com/nha-org/site_perso/pull/7#issuecomment-1"),
            [PR]
        );
        assert!(same("https://github.com/NHA/Site_Perso/pull/7", PR));
        assert!(!same("https://github.com/nha-org/klar/pull/7", PR));
        let klar = ReviewClaim {
            pr: "https://github.com/nha-org/klar/pull/7".into(),
            ..claimed("other", None)
        };
        let old_owner = ReviewClaim {
            pr: "https://github.com/nha/site_perso/pull/7".into(),
            ..claimed("helper", None)
        };
        let stored = [klar, old_owner.clone()];
        let ends = change(
            &Asked::of(&args("review release 7"), &|| None).unwrap(),
            None,
            &stored,
            &|_| None,
        )
        .ends;
        // a bare number ends that number's claims in every repository, as the older app's release did (/code-review of PR 1651)
        let klar = ("other".to_string(), stored[0].pr.clone());
        assert_eq!(ends, [klar, ("helper".to_string(), old_owner.pr.clone())]);
        let second = judge(
            &Asked::of(&args(&format!("review claim {PR} b")), &|| None).unwrap(),
            None,
            &stored,
        );
        assert!(
            second
                .expect("Rust's")
                .unwrap_err()
                .contains("helper holds a live claim")
        );
        assert_eq!(held_prs(&stored), [7]);
    }

    /// A claim is a row per holder and link in the plugin's log: a claim again renews it, an end drops it, and one written past
    /// the 6 hours is not read.
    #[dash_test]
    fn a_claim_stands_in_the_log_until_it_ends_or_lapses() {
        let mut log = dash_core::MemoryLog::default();
        let c = |pr: &str| ReviewClaim {
            pr: pr.into(),
            ..claimed("helper", Some("w1:p1"))
        };
        let both = Change {
            claims: vec![
                c("https://x/pull/7"),
                c("https://x/pull/8"),
                c("https://x/pull/7"),
            ],
            ..Change::default()
        };
        write(&mut log, &both).expect("claimed");
        let end = Change {
            ends: vec![("helper".into(), "https://x/pull/8".into())],
            ..Change::default()
        };
        write(&mut log, &end).expect("ended");
        // MemoryLog's stamps count up from 1, each below 1 << 16: all read as its millisecond 0
        let read = stored(&log, LANE_HOLD_MS - 1).expect("read");
        assert_eq!(read, [c("https://x/pull/7")]);
        assert_eq!(
            stored(&log, LANE_HOLD_MS + 1).expect("read"),
            [],
            "past 6 hours"
        );
    }

    /// Rust alone refuses a second claimer, a claim beside merge on green and a PM reviewer's id, as the older app did; the
    /// claimer itself claims again; a release ends the claims on a PR, merge on green's only with --all.
    #[dash_test]
    fn rust_alone_judges_a_claim_and_a_release() {
        let held = [claimed("helper", Some("w1:p1"))];
        let ask = |w: &str| {
            judge(&Asked::of(&args(w), &|| None).expect("ours"), None, &held).expect("Rust's")
        };
        assert_eq!(
            ask(&format!("review claim {PR} other")).unwrap_err(),
            format!(
                "review claim: helper holds a live claim on {PR}: dash review release {PR} ends it"
            )
        );
        let (again, said) = ask(&format!("review claim {PR} helper")).expect("its own");
        assert_eq!(again.claims.len(), 1);
        assert!(said.ends_with("for 6 hours\n"), "{said}");
        assert!(
            ask(&format!("review claim {PR} review-x"))
                .unwrap_err()
                .contains("name the helper")
        );
        assert!(
            ask("review claim soon helper")
                .unwrap_err()
                .contains("holds no PR link")
        );
        assert!(
            ask("review claim 7 helper")
                .unwrap_err()
                .contains("holds no PR link")
        );
        let handed = [
            ReviewClaim {
                pusher: Some("review-x".into()),
                ..claimed(MERGE_ON_GREEN, None)
            },
            claimed("helper", None),
        ];
        let release = |w: &str| {
            change(
                &Asked::of(&args(w), &|| None).expect("ours"),
                None,
                &handed,
                &|_| None,
            )
            .ends
        };
        assert_eq!(
            release("review release 7"),
            [("helper".to_string(), PR.to_string())]
        );
        assert_eq!(release("review release #7 --all").len(), 2);
        assert_eq!(release("review release helper").len(), 1);
        let on_mog = judge(
            &Asked::of(&args(&format!("review claim {PR} helper")), &|| None).unwrap(),
            None,
            &handed,
        );
        assert!(
            on_mog
                .expect("Rust's")
                .unwrap_err()
                .contains("merge on green CI")
        );
    }

    /// A hand-over records merge on green with its reviewer as the pusher, a bare number as the caller's repository's link, and
    /// ends the hander's own claim; a helper's or the lane's hand-over names the PR's task's reviewer, whom the older app
    /// resumes on red; `task pr` names the new pusher of a handed PR alone, by link or by number (/code-review of PR 1651).
    #[dash_test]
    fn merge_on_green_s_pushers_are_its_reviewer_then_the_new_owner() {
        // the hander's claim stored under an old owner's link ends too (/code-review medium of PR 1651)
        let old = ReviewClaim {
            pr: "https://github.com/nha/site_perso/pull/7".into(),
            ..claimed("review-x", None)
        };
        let handed = change(
            &Asked::of(&args("review handover 7 review-x"), &|| None).unwrap(),
            Some(("w1:p1".into(), "s".into())),
            std::slice::from_ref(&old),
            &|_| None,
        );
        assert_eq!(handed.ends, [("review-x".to_string(), old.pr.clone())]);
        let klar = Asked::of(&args("review handover #12 review-x"), &|| {
            Some("nha-org/klar".into())
        });
        assert!(
            matches!(klar, Some(Asked::Handover { ref pr, .. }) if pr == "https://github.com/nha-org/klar/pull/12")
        );
        let task_of = |l: &str| (l == PR).then(|| "foo".to_string());
        for rid in ["helper", LANE] {
            let asked = Asked::of(&args(&format!("review handover {PR} {rid}")), &|| None).unwrap();
            let by = change(&asked, None, &[claimed(rid, None)], &task_of);
            assert_eq!(pusher(&by.claims[0]), "review-foo", "{rid}");
            assert_eq!(by.ends, [(rid.to_string(), PR.to_string())]);
        }
        let c = &handed.claims[0];
        assert_eq!(
            (c.who.as_str(), c.pr.as_str(), pusher(c), c.pane.as_ref()),
            (MERGE_ON_GREEN, PR, "review-x", None)
        );
        let live = [c.clone(), claimed("helper", None)];
        let moved = change(
            &Asked::of(&args(&format!("task pr {PR} rust-y")), &|| None).unwrap(),
            None,
            &live,
            &|_| None,
        );
        assert_eq!(
            moved.claims.iter().map(pusher).collect::<Vec<_>>(),
            ["rust-y"]
        );
        let by_number = Asked::of(&args("task pr 7 rust-y"), &|| None).expect("a number is a PR");
        assert_eq!(change(&by_number, None, &live, &|_| None), moved);
        // a second hand-over writes nothing: merge on green's 6 hours never start again (/code-review medium of PR 1651)
        let again = Asked::of(&args(&format!("review handover {PR} review-x")), &|| None).unwrap();
        assert!(change(&again, None, &live, &|_| None).claims.is_empty());
        // a new owner with no hand-over holds a claim of its own, so it pushes alone (/code-review medium of PR 1651)
        let owner = change(&by_number, None, &[claimed("helper", None)], &|_| None);
        assert_eq!(
            owner
                .claims
                .iter()
                .map(|c| (c.who.as_str(), c.pr.as_str()))
                .collect::<Vec<_>>(),
            [("rust-y", PR)]
        );
        assert!(Asked::of(&args("task pr rust-y"), &|| None).is_none());
        assert!(hits(
            "https://github.com/nha/site_perso/pull/7/",
            "site_perso",
            7
        ));
        assert!(!hits(PR, "klar", 7));
    }
}
