//! Section 20b's orchestrator, through the ports alone. `next` frees what a crash left and picks; `run_task` carries one pick
//! from its claim to a merged PR: worktree, pane, the agent, its words, its turns judged, CI, the merge.
use dash_core::tasks::{
    Agent, Base, Checks, Claim, Forge, Launcher, Limit, Outcome, Pick, Plan, Policy, Pr, Queue,
    Queued, Run, Screen, Seen, Standing, Step, Task, TaskList, TaskStore, Terminal, Verdict,
    at_cap, handed_back, plan,
};
use std::thread::sleep;
use std::time::{Duration, Instant};

pub struct Ports<'a> {
    pub tasks: &'a mut dyn TaskStore,
    pub list: &'a dyn TaskList,
    pub terminal: &'a dyn Terminal,
    pub launcher: &'a dyn Launcher,
    pub agent: &'a dyn Agent,
    pub forge: &'a dyn Forge,
    pub policy: &'a dyn Policy,
    /// Told each step's name and time, for whoever keeps them.
    pub step: &'a dyn Fn(&str, Duration),
    /// The machine's free memory in percent, read at each pick; `None` unread.
    pub free_memory: &'a dyn Fn() -> Option<u8>,
}

/// The steps of one pass, each timed from the end of the last. A step that ends the pass by `?` names no time; one whose
/// failure the pass carries on past names its own.
struct Lap<'a> {
    step: &'a dyn Fn(&str, Duration),
    at: Instant,
}

impl<'a> Lap<'a> {
    fn new(step: &'a dyn Fn(&str, Duration)) -> Self {
        let at = Instant::now();
        Self { step, at }
    }

    fn done(&mut self, name: &str) {
        (self.step)(name, self.at.elapsed());
        self.at = Instant::now();
    }
}

/// `limit_retry` is the least time between two words to a session a limit stopped. `start_limit` is for its prompt and for the terminal to list it after, `limit` the time after which a session still at work
/// is judged, `stuck_limit` for one left waiting (idle or asking) with no outcome, `stop_polls` the looks it has to leave once
/// told, and for one unlisted to count as gone. `ci_poll` and `ci_limit` are for a PR's checks, `judge_poll` for a judge with
/// no answer yet, `patient_poll` for a session waiting on someone.
#[derive(Clone, Copy)]
pub struct Pace {
    pub poll: Duration,
    pub start_limit: Duration,
    pub limit: Duration,
    pub stuck_limit: Duration,
    pub stop_polls: u32,
    pub ci_poll: Duration,
    pub ci_limit: Duration,
    pub judge_poll: Duration,
    pub patient_poll: Duration,
    pub limit_retry: Duration,
}

impl Pace {
    /// A test's: no wait between looks, five seconds before a limit.
    pub const TEST: Pace = Pace {
        poll: Duration::ZERO,
        start_limit: Duration::from_secs(5),
        limit: Duration::from_secs(5),
        stuck_limit: Duration::from_secs(5),
        stop_polls: 3,
        ci_poll: Duration::ZERO,
        ci_limit: Duration::from_secs(5),
        judge_poll: Duration::ZERO,
        patient_poll: Duration::ZERO,
        limit_retry: Duration::ZERO,
    };
}

/// The sessions a seat counts: those listed, and each run claimed and not listed yet, as a start takes minutes and each look
/// would start one more. A failed look counts it, the side that stays within his seats. A claim whose session left and whose
/// pane closed (`TaskStore::left`: a met run driving its PR alone) counts none.
pub fn seats_taken(terminal: &dyn Terminal, claims: &[Claim]) -> Result<usize, String> {
    let listed = |claim: &&Claim| {
        let (Some(pane), Some(session)) = (&claim.pane, &claim.session) else {
            return claim.pane.is_none() && claim.session.is_some();
        };
        let seen = terminal.agent(pane);
        seen.is_ok_and(|s| s.is_some_and(|s| s.session.as_ref() == Some(session)))
    };
    let starting = claims.iter().filter(|claim| !listed(claim)).count();
    Ok(terminal.sessions()? + starting)
}

/// What to do now: claims a crash left are freed first, what they left closed, one out of tries told about; then `plan` picks
/// among what no run holds and nothing it waits on holds back, each task the list opened again starting over.
pub fn next(ports: &mut Ports) -> Result<Pick, String> {
    let mut lap = Lap::new(ports.step);
    // The first crashed claim whose session still works: picked once every other crashed claim is dealt with in this look.
    let mut adopt = None;
    for crashed in ports.tasks.crashed()? {
        // Its PR, read once for the adoption and the merge below.
        let mut read = None;
        // Its session only: the pane id may be another's by now (herdr reuses them, and a reboot starts them over). A look or a
        // close that failed, or an agent listed with no id yet, leaves the row claimed, so no retry starts beside a session still
        // at work: the next look tries again.
        // A claim held is flagged at each look, as nothing else moves it: the app keeps one flag of the same words a day.
        // ponytail: a crash in the seconds before a session's id showed, or after a /clear gave it another, leaves that session as
        // any other's, with a retry beside it, flagged. Only a saved id tells it from a reused pane, and a reboot reuses them all.
        if let Some(pane) = &crashed.pane {
            match ports.terminal.agent(pane) {
                Ok(Some(seen)) if seen.session.is_none() => {
                    let held = "its run is gone and its pane lists an agent with no id: the claim holds until it shows one";
                    let _ = ports.list.flag(&crashed.id, held);
                    continue;
                }
                // Its own session, still at work: a new run watches it on, never closes it and starts over (a restart left
                // merge-gate's run on second's limit menu, flagged as a question, while default had room, 2026-11-09). Only on a
                // read that shows no merge: a failed one proves nothing, and a merged PR's session is closed as below. Its last
                // try is closed as any crash's.
                Ok(Some(seen)) if seen.session == crashed.session => {
                    let pr = ports.forge.pr(&crashed.id);
                    let open = matches!(&pr, Ok(pr) if pr.as_ref().is_none_or(|pr| !pr.merged));
                    if open && ports.policy.retry(crashed.attempts) {
                        adopt.get_or_insert_with(|| crashed.id.clone());
                        continue;
                    }
                    read = Some(pr);
                    if let Err(e) = ports.terminal.close(pane) {
                        let held = format!("its run is gone and its pane will not close: {e}");
                        let _ = ports.list.flag(&crashed.id, &held);
                        continue;
                    }
                }
                Ok(Some(_)) => {
                    let what = "its run is gone and its pane lists a session it did not save: a retry starts. If that is its own after a /clear, two work in one worktree";
                    let _ = ports.list.flag(&crashed.id, what);
                }
                Ok(None) => {}
                Err(_) => continue,
            }
        }
        let freed = ports.tasks.free(&crashed.id)?;
        if freed {
            // Its run may have crashed in a wait: no one is left to say it is over.
            let _ = ports.list.waiting(&crashed.id, None);
        }
        // Once freed, so a claim another process still holds costs no GitHub read at each look: a merge after its run died is that
        // run's end, never a try more. ponytail: a run that died before its first commit on a reopen that kept a merged PR's branch
        // reads as merged too; only its run knew that PR came before it.
        let merged = freed.then(|| match read {
            Some(pr) => pr.ok().flatten().filter(|pr| pr.merged).map(|pr| pr.url),
            None => merged_since(ports, &crashed.id, None),
        });
        let merged = merged.flatten();
        if let Some(url) = &merged {
            let id = &crashed.id;
            let run = Run {
                pane: crashed.pane.clone(),
                session: crashed.session.clone(),
                outcome: Outcome::Merged(url.clone()),
            };
            ports.tasks.record(id, &run)?;
            // A build or a restack in the look would hold every other task's look for minutes: the next merge's run does both.
            // On its row before `told`, which may end this look: the next one never comes back to a finished row.
            let left = "merged after its run died: no deploy or restack ran for it; the next merge's run does both";
            let _ = ports.tasks.deployed(id, &format!("none: {left}"));
            let _ = ports.forge.clean(id);
            if ports.list.merged(id, url).is_ok() {
                ports.tasks.told(id)?;
            }
            let _ = ports.list.flag(id, left);
            continue;
        }
        if freed && !ports.policy.retry(crashed.attempts) {
            let (id, attempts) = (&crashed.id, crashed.attempts);
            let why = format!("its run crashed, {attempts} tries");
            let run = Run {
                pane: crashed.pane.clone(),
                session: crashed.session.clone(),
                outcome: Outcome::Failed(why.clone()),
            };
            ports.tasks.record(id, &run)?;
            if ports.list.ended(id, &why).is_ok() {
                ports.tasks.told(id)?;
            }
        }
    }
    lap.done("free_crashed");
    if let Some(id) = adopt {
        return Ok(Pick::Adopt(id));
    }
    // Rust's own copy of the list, synced before the pass: nothing here asks the older app what to pick.
    let queue = ports.tasks.queue()?;
    lap.done("queue");
    for task in &queue.tasks {
        // Held at the cap: what its old run said must not close what he just opened. The app keeps one flag of the same words a day.
        if let Some(at) = task.reopened_ms
            && ports.tasks.reopen_held(&task.id, at)?
        {
            let what = "its reopen waits: Rust's open tasks are at their cap";
            let _ = ports.list.flag(&task.id, what);
            continue;
        }
        match ports.tasks.status(&task.id)?.as_deref() {
            None | Some("open") => {}
            // Finished, and the list not told yet, or it would no longer list it. Told again at each look.
            _ => {
                if let Some(outcome) = ports.tasks.untold(&task.id)?
                    && tell_list(&task.id, &outcome, ports.list).is_ok()
                {
                    ports.tasks.told(&task.id)?;
                }
            }
        }
    }
    let rows = ports.tasks.standing()?;
    lap.done("pickable");
    // As a reopen held at the cap, and the app keeps one flag of the same words a day: nothing else says work stopped here.
    if let Some(full) = at_cap(&rows) {
        let new = |t: &&Queued| !rows.iter().any(|r| r.id == t.id);
        for task in queue.tasks.iter().filter(new) {
            let _ = ports.list.flag(&task.id, &full);
        }
    }
    // A hand-back that should not start leaves this look's queue, and the plan is made again: one PR read never stalls the
    // other tasks (/code-review of 6305f42). Each round drops one task, so it ends.
    let mut queue = queue;
    loop {
        let pick = plan_now(ports, queue.clone(), &rows)?.next;
        match still_handed(ports, &rows, &pick)? {
            Some(id) => queue.tasks.retain(|t| t.id != id),
            None => return Ok(pick),
        }
    }
}

/// A hand-back about to start whose PR moved since it was handed back: its CI read again first, as comments-board-pill's run
/// found the fix pushed and had nothing to do (task rust-pick-handbacks-first). A new head still needing a fix starts, its
/// first words read again by `drive`. A new head needing none, or a merge, ends its row again with its PR, flagged: `hand_backs`
/// watches it, and judges a merge again. A head unmoved, or no PR, starts as picked. `Some(id)`: not to start in this look,
/// a PR that could not be read included, flagged.
fn still_handed(
    ports: &mut Ports,
    rows: &[Standing],
    pick: &Pick,
) -> Result<Option<String>, String> {
    let Pick::Start { id, .. } = pick else {
        return Ok(None);
    };
    let Some(at) = handed_back(rows, id) else {
        return Ok(None);
    };
    let skip = |ports: &Ports, why: &str| {
        let _ = ports.list.flag(id, why);
        Ok(Some(id.clone()))
    };
    let pr = match ports.forge.pr(id) {
        Ok(Some(pr)) if pr.merged || pr.head != at => pr,
        Ok(_) => return Ok(None),
        Err(e) => {
            return skip(
                ports,
                &format!("handed back, its PR not read again, not started: {e}"),
            );
        }
    };
    let (head, url) = (&pr.head, &pr.url);
    let why = match pr.merged {
        true => format!("{url} merged since it was handed back at {at}: no session started"),
        false => match needs_fix(ports.forge, &pr) {
            Err(e) => {
                return skip(
                    ports,
                    &format!("handed back, its PR not read again, not started: {e}"),
                );
            }
            Ok(Some(_)) => return Ok(None),
            Ok(None) => format!(
                "its PR head moved from {at} to {head} since it was handed back, and {url} needs no fix now: no session started"
            ),
        },
    };
    let run = Run {
        pane: None,
        session: None,
        outcome: Outcome::Ended(why.clone()),
    };
    ports.tasks.record(id, &run)?;
    skip(ports, &why)
}

/// What a new session must fix on `pr` first; `None`: nothing (green, CI running, behind main alone, which `land` updates). Red
/// and behind is still red, which no one else would drive (`gate` reads main first and stops there). Err: no verdict.
fn needs_fix(forge: &dyn Forge, pr: &Pr) -> Result<Option<String>, String> {
    Ok(match gate(forge, pr)? {
        Gate::Fix(words) => Some(words),
        Gate::Behind => match forge.checks(pr)? {
            Checks::Red(jobs) => Some(format!("CI is red on {}: {jobs}. Fix it and push", pr.url)),
            Checks::Green | Checks::Pending => None,
        },
        Gate::Ready | Gate::Wait => None,
    })
}

/// The plan of this moment, `next`'s last step and all `dash orch next` asks: the seats taken, each account's limits and the
/// free memory read now, then `plan`. Changes nothing, so its answer is the pick a pass would make from the same list.
pub fn plan_now(ports: &Ports, queue: Queue, rows: &[Standing]) -> Result<Plan, String> {
    let mut lap = Lap::new(ports.step);
    let sessions = seats_taken(ports.terminal, &ports.tasks.claims()?)?;
    lap.done("sessions");
    // A failed read is no spend known: its Pick still holds to the seats.
    // ponytail: counts sessions, not the subagents Admission adds; a seat's worth over, only while one runs.
    let limits = ports.agent.limits().unwrap_or_default();
    lap.done("limits");
    let free = (ports.free_memory)();
    let plan = plan(ports.policy, queue, rows, sessions, limits, free);
    lap.done("pick");
    Ok(plan)
}

/// Hand-backs of one task before it is left flagged: a reopen starts its tries over, so only this ends a PR its sessions keep
/// pushing red.
const HAND_BACKS: u32 = 3;

/// Each run that ended with its PR open and still listed goes back to its task once that PR conflicts with main or its CI is red:
/// reopened as `dash task reopen` does (@nha, 2026-11-09: « Your loop should do that same reopen on a conflicting PR from now on »),
/// so a new run starts in its worktree, its first words saying what (`drive`), rather than a PR nobody drives. Once per PR head: a
/// session that moved nothing is not handed the same head again. A head only behind main is not handed back: main moves every few
/// minutes, and `land` merges main into a met run's branch itself. An ended run still listed whose PR merged since is judged again (`rejudge`).
/// What was handed back or judged again, with the words. A failed read or write for one is no verdict: a later pass reads it
/// again.
pub fn hand_backs(ports: &mut Ports) -> Result<Vec<(String, String)>, String> {
    let listed = ports.tasks.queue()?.tasks;
    let mut handed = vec![];
    for row in ports.tasks.standing()? {
        let ended = row.status == "finished" && row.outcome.as_deref() == Some("ended");
        if !ended {
            continue;
        }
        // Unlisted, the list closed it: judging it again reopened three tasks met and done that morning (2026-11-09).
        if !listed.iter().any(|t| t.id == row.id) {
            continue;
        }
        let Ok(pr) = ports.forge.pr(&row.id) else {
            continue;
        };
        let done = match pr {
            Some(pr) if pr.merged => rejudge(ports, &row, &pr),
            pr => hand_back(ports, &row, pr),
        };
        match done {
            Ok(Some(words)) => handed.push((row.id, words)),
            // Said where it is read: the app keeps one flag of the same words a day. A failed read is no verdict, said by no flag.
            Err(Held(why)) => {
                let _ = ports.list.flag(&row.id, &why);
            }
            Ok(None) | Err(Unread) => {}
        }
    }
    Ok(handed)
}

/// Why a hand-back did not happen: `Held`, something only whoever coordinates moves; `Unread`, a read or write that failed.
enum NoHandBack {
    Held(String),
    Unread,
}
use NoHandBack::{Held, Unread};

/// `handed` as the store keeps it: the PR head last handed back, then how many hand-backs so far.
fn handed_in(handed: Option<&str>) -> (Option<&str>, u32) {
    let Some((head, n)) = handed.and_then(|h| h.split_once(' ')) else {
        return (handed, u32::from(handed.is_some()));
    };
    (Some(head), n.parse().unwrap_or(HAND_BACKS))
}

/// A session still in its last pane, as a stop that failed leaves it (or one this pane id now holds, not yet named), holds a
/// row: no run starts beside it.
fn pane_free(ports: &Ports, row: &Standing, what: &str) -> Result<(), NoHandBack> {
    let Some(pane) = &row.pane else {
        return Ok(());
    };
    let seen = ports.terminal.agent(pane).map_err(|_| Unread)?;
    if seen.is_some_and(|seen| seen.session.is_none() || seen.session == row.session) {
        return Err(Held(format!(
            "{what}: a session is still in its last pane {pane}"
        )));
    }
    Ok(())
}

/// `None`: its PR needs nothing a new session should start for (none, green, behind main alone, CI still running), or it was
/// handed back at this head already. `pr`: its newest, read once by the pass, never a merged one.
fn hand_back(
    ports: &mut Ports,
    row: &Standing,
    pr: Option<Pr>,
) -> Result<Option<String>, NoHandBack> {
    let id = &row.id;
    pane_free(ports, row, "not handed back")?;
    let Some(pr) = pr else {
        return Ok(None);
    };
    let (head, n) = handed_in(row.handed.as_deref());
    if head == Some(pr.head.as_str()) {
        return Ok(None);
    }
    // Behind alone moves with main, so it starts no run.
    let Some(words) = needs_fix(ports.forge, &pr).map_err(|_| Unread)? else {
        return Ok(None);
    };
    if n >= HAND_BACKS {
        return Err(Held(format!(
            "not handed back: {n} hand-backs left it so: {words}"
        )));
    }
    // The reopen first: a head written as handed before a reopen that failed would never be handed back.
    ports.list.reopen(id).map_err(|_| Unread)?;
    let handed = format!("{} {}", pr.head, n + 1);
    ports.tasks.handed(id, &handed).map_err(|_| Unread)?;
    let _ = ports
        .list
        .flag(id, &format!("handed back to a new run: {words}"));
    Ok(Some(words))
}

/// A run given up on whose PR merged after, whoever merged it: judged again, as four were left ended with their PRs merged
/// (@nha, 2026-11-05: « help me build what is important fast with minimal involvement »). Met, it lands as a merge; not met,
/// its task is reopened with the reason; blocked, flagged, as a new run would meet the same block; not ours, its row is merged
/// here and the list keeps its own word. Each is
/// flagged on its card, its verdict on the row for `dash task show`. Once per merged head (`rejudged`, kept across a reopen): a
/// new run that ends on the same old PR is not judged on it again. `None`: that head was judged already.
/// ponytail: no restack or deploy runs for such a merge, as for one after a crash: the next merge's run does both.
fn rejudge(ports: &mut Ports, row: &Standing, pr: &Pr) -> Result<Option<String>, NoHandBack> {
    let (id, url) = (&row.id, &pr.url);
    let judged = row.rejudged.as_deref().and_then(|r| r.split_once(' '));
    if judged.is_some_and(|(head, _)| head == pr.head) {
        return Ok(None);
    }
    // Before the judge, a model's call: a reopen would start a run beside that session.
    pane_free(ports, row, "not judged again")?;
    let said = format!("its pull request {url} merged after its run ended");
    let verdict = ports.list.judge(id, &said).map_err(|_| Unread)?;
    let words = match verdict {
        Verdict::Met | Verdict::NotOurs(_) => {
            let run = Run {
                pane: row.pane.clone(),
                session: row.session.clone(),
                outcome: Outcome::Merged(url.clone()),
            };
            // A record is untold: `next` tells the list again at each look while it stays listed.
            ports.tasks.record(id, &run).map_err(|_| Unread)?;
            let _ = ports.forge.clean(id);
            let words = match verdict {
                Verdict::NotOurs(why) => {
                    let _ = ports.tasks.told(id);
                    format!("not ours ({why}), recorded merged: {said}")
                }
                _ => {
                    // Its own merge, unlike one not ours.
                    let none = "none: merged after its run ended, no deploy ran for it; the next merge's run does it";
                    let _ = ports.tasks.deployed(id, none);
                    if ports.list.merged_later(id, url).is_ok() {
                        let _ = ports.tasks.told(id);
                    }
                    format!("met: {said}")
                }
            };
            let _ = ports.list.flag(id, &format!("judged again, {words}"));
            words
        }
        Verdict::Blocked(needs) => {
            let words = format!("blocked, not reopened: {said}, and it needs {needs}");
            let _ = ports.list.flag(id, &format!("judged again, {words}"));
            words
        }
        Verdict::NotMet(why) => {
            ports.list.reopen(id).map_err(|_| Unread)?;
            let words = format!("not met, reopened: {said}, but {why}");
            let _ = ports.list.flag(id, &format!("judged again, {words}"));
            words
        }
    };
    let marked = format!("{} {words}", pr.head);
    ports.tasks.rejudged(id, &marked).map_err(|_| Unread)?;
    Ok(Some(words))
}

/// What a run's drive leaves, however it ends: its pane, the session it adopted there, and, once it read its PR, the URL of that
/// PR if it merged before the run began (`None`: not read).
#[derive(Default)]
struct Left {
    pane: Option<String>,
    session: Option<String>,
    before: Option<Option<String>>,
}

/// Its PR's URL once merged, unless it merged before this run (`before`: a reopen keeps a merged PR's branch). A failed read is
/// no merge.
fn merged_since(ports: &Ports, id: &str, before: Option<&str>) -> Option<String> {
    let pr = ports.forge.pr(id).ok().flatten().filter(|pr| pr.merged)?;
    (before != Some(pr.url.as_str())).then_some(pr.url)
}

/// The list told how a finished run ended: a merge closes its task, anything else leaves its PR to the list's own pass.
fn tell_list(id: &str, outcome: &Outcome, list: &dyn TaskList) -> Result<(), String> {
    match outcome {
        Outcome::Merged(pr) => list.merged(id, pr),
        Outcome::Ended(why) | Outcome::Failed(why) => list.ended(id, why),
        Outcome::Done(_) | Outcome::Blocked(_) | Outcome::Limited(_) | Outcome::Offline(_) => {
            Ok(())
        }
    }
}

/// One task's run, recorded whatever it ends as. Err: it was never claimed, or its record or the close failed.
pub fn run_task(
    id: &str,
    account: Option<&str>,
    ports: &mut Ports,
    pace: Pace,
) -> Result<Run, String> {
    let mut lap = Lap::new(ports.step);
    let attempts = ports.tasks.claim(id)?;
    lap.done("claim");
    carry(id, account, attempts, Left::default(), None, ports, pace)
}

/// A crashed run's session, still at work in its pane (`Pick::Adopt`), watched on by this run: its pane, session and login
/// kept, no new pane, start or brief, then carried to its end as any run. A run that dies anywhere, a restart's included, leaves
/// its session to the next look rather than close it and start over.
pub fn adopt_task(id: &str, ports: &mut Ports, pace: Pace) -> Result<Run, String> {
    let held = ports.tasks.hold(id)?;
    // Before anything that ends a run: one that cannot read its folder leaves the session at work and the row claimed, as a
    // crash does, for the next look.
    let folder = ports
        .forge
        .worktree_as_is(id)
        .map_err(|e| format!("worktree: {e}"))?;
    let left = Left {
        pane: held.pane,
        session: held.session,
        before: None,
    };
    let account = held.account.as_deref();
    carry(id, account, held.attempts, left, Some(folder), ports, pace)
}

/// One run from its claim or hold to its end: driven, recorded or released, its pane closed. `adopted`: the folder of a session
/// already at work in `left`'s pane.
fn carry(
    id: &str,
    account: Option<&str>,
    attempts: u32,
    mut left: Left,
    adopted: Option<String>,
    ports: &mut Ports,
    pace: Pace,
) -> Result<Run, String> {
    let driven = drive(id, account, attempts, adopted, ports, pace, &mut left);
    let Left {
        pane,
        session,
        before,
    } = left;
    let mut lap = Lap::new(ports.step);
    let outcome = driven.unwrap_or_else(Outcome::Failed);
    // A session gone after its PR merged, whoever merged it (a pane closed by hand, a crash in the seconds after), is a merge:
    // recorded, its pane closed below, never a second try (rust-skeleton-live's was retried after a merge, 2026-11-09).
    // Only once the run read its PR: a drive that failed before that cannot tell an old merge (a reopen) from its own.
    let outcome = match (outcome, &before) {
        (outcome @ (Outcome::Failed(_) | Outcome::Ended(_)), Some(before)) => {
            merged_since(ports, id, before.as_deref()).map_or(outcome, Outcome::Merged)
        }
        (outcome, _) => outcome,
    };
    let retry = matches!(outcome, Outcome::Failed(_)) && ports.policy.retry(attempts);
    let mut run = Run {
        pane,
        session,
        outcome,
    };
    // An end for good is recorded first: stop may take minutes on a hung terminal, and a run killed in them has still said how
    // it ended. A retry waits: releasing frees the lock, and the next look would start a second session in this worktree.
    let (mut recorded, mut told) = (Ok(()), Ok(()));
    if !retry {
        recorded = ports.tasks.record(id, &run);
        told = tell_list(id, &run.outcome, ports.list).and_then(|()| ports.tasks.told(id));
    }
    lap.done("record");
    let closed = match run.pane.clone() {
        Some(pane) => stop(id, &pane, &mut run.session, ports, pace),
        None => Ok(()),
    };
    // A stop that failed may have left its session at work: the row stays claimed, its lock leaves with this process, and `next`
    // closes what is left before anything retries. A pane that still lists a session after its stop holds one this run cannot name
    // (a /clear gives Claude another id): it may be this work, so the run ends for good, flagged, rather than start beside it.
    let mut closed = closed;
    if retry && closed.is_ok() {
        let listed = run.pane.as_deref().map(|pane| ports.terminal.agent(pane));
        match listed {
            Some(Err(e)) => closed = Err(e),
            Some(Ok(Some(_))) => {
                let (_, said) = run.outcome.parts();
                let why = format!(
                    "{said}; its pane lists a session this run cannot name, so no retry starts beside it"
                );
                run.outcome = Outcome::Failed(why);
                recorded = ports.tasks.record(id, &run);
                told = tell_list(id, &run.outcome, ports.list).and_then(|()| ports.tasks.told(id));
            }
            _ => recorded = ports.tasks.release(id, &run),
        }
    }
    lap.done("close");
    // Its build output goes before the build, which needs the disk, even when the pane stayed open: no session's work is in it.
    // The rest of its worktree goes only once its session is stopped, as one still at work there would lose its files. A failed
    // deploy is flagged at once: the run's own error reaches only its log, and an earlier one would hide it.
    let merged = matches!(run.outcome, Outcome::Merged(_));
    // Before `leave`, which needs its worktree for the read of its PR's head: what stacked on its branch would conflict with the
    // squash. Each PR restacked, or not, is said on this task's card.
    // A branch another run's session is at work on is left to it.
    if merged && let Ok(Some(pr)) = ports.forge.pr(id) {
        let claims = ports.tasks.claims().unwrap_or_default();
        let others = claims.iter().filter(|c| c.id != id);
        let busy: Vec<String> = others.map(|c| format!("task/{}", c.id)).collect();
        let said = ports
            .forge
            .restack(&pr, &busy)
            .unwrap_or_else(|e| vec![format!("no restack: {e}")]);
        for words in said {
            let _ = ports.list.flag(id, &format!("after its merge: {words}"));
        }
    }
    lap.done("restack");
    let left = match (merged, &closed) {
        (true, Ok(())) => ports.forge.clean(id).and_then(|()| ports.forge.leave(id)),
        (true, Err(_)) => ports.forge.clean(id),
        (false, _) => Ok(()),
    };
    lap.done("leave");
    // Its row says what became of main (#1265 merged at 09:09Z, live stayed on the commit before, and nothing said so): written
    // first as begun, so a run that dies in the build leaves that word, then what serves or the error, one line: make's output
    // is the flag's. A write that fails is no failed run: the deploy itself stands.
    let mut deployed = Ok(());
    if merged {
        let _ = ports
            .tasks
            .deployed(id, "begun, no end recorded: its run stopped in the deploy");
        let done = ports.forge.deploy();
        let done = done.map_err(|e| format!("{id} merged, but main is not live: {e}"));
        let words = match &done {
            Ok(said) => said.clone(),
            Err(e) => format!("failed: {e}"),
        };
        let words: Vec<&str> = words.split_whitespace().collect();
        let words: String = words.join(" ").chars().take(DEPLOY_WORDS).collect();
        let _ = ports.tasks.deployed(id, &words);
        if let Err(e) = &done {
            let _ = ports.list.flag(id, e);
        }
        deployed = done.map(drop);
    }
    lap.done("deploy");
    recorded?;
    // `next` tells it again at each look.
    told.map_err(|e| format!("{id} is recorded, but the list was not told yet: {e}"))?;
    closed.map_err(|e| format!("{id}: its pane stayed open: {e}"))?;
    left.map_err(|e| format!("{id} merged, but its worktree stays: {e}"))?;
    deployed?;
    Ok(run)
}

/// `dash task close` on a run this orchestrator holds (rust-task-close-rust-runs; @nha, 2026-11-09: « fix the root cause not the
/// surface one-time thing. »): refused while it starts, while its agent works or its pane lists a session it cannot name, while
/// its worktree holds work no remote has, or while an unfinished task waits on it; else its run's process stopped, its agent told
/// to leave and its pane closed, its build output gone, its row ended for good, never retried. Err `NOT_HELD` when no run of this
/// store holds it: the older app's close is the one to ask.
/// ponytail: a nudge its run types in the milliseconds between the look and the stop reaches an agent told to leave next.
pub fn close(id: &str, why: &str, ports: &mut Ports, pace: Pace) -> Result<String, String> {
    let claims = ports.tasks.claims()?;
    let Some(claim) = claims.into_iter().find(|c| c.id == id) else {
        return Err(format!("{id}: {NOT_HELD}"));
    };
    // No pane yet: its launch is in flight, and a pane it opens after the stop would be nobody's (/code-review of baa197583).
    let Some(pane) = claim.pane.clone() else {
        return Err(format!(
            "{id}: refused, its run opens no pane yet: still starting"
        ));
    };
    let seen = (ports.terminal.agent(&pane))
        .map_err(|e| format!("{id}: refused, its pane {pane} gave no look: {e}"))?;
    let ours = match (&seen, &claim.session) {
        (None, _) => false,
        (Some(seen), Some(saved)) if seen.session.as_ref() == Some(saved) => {
            if !seen.idle && !seen.asking {
                return Err(format!(
                    "{id}: refused, its agent works in pane {pane}: close it once its turn ends"
                ));
            }
            true
        }
        // A session it has not saved yet (still starting), or another id (a /clear's, or another's in a reused pane id): it may be
        // this work, as `run_task` reads it, so nothing is stopped beside it (/code-review of baa197583).
        (Some(_), _) => {
            return Err(format!(
                "{id}: refused, pane {pane} lists a session its run cannot name: still starting, or a /clear's; close it once you know whose"
            ));
        }
    };
    let rows = ports.tasks.standing()?;
    let waiting = rows
        .iter()
        .filter(|r| r.status != "finished" && r.waits.iter().any(|w| w == id));
    let waiting: Vec<&str> = waiting.map(|r| r.id.as_str()).collect();
    if !waiting.is_empty() {
        let names = waiting.join(", ");
        return Err(format!(
            "{id}: refused, {names} waits on it: dash task waits changes that first"
        ));
    }
    if let Some(what) = (ports.forge.unsaved(id))
        .map_err(|e| format!("{id}: refused, its worktree gave no look: {e}"))?
    {
        return Err(format!(
            "{id}: refused, its worktree holds {what}: push it first"
        ));
    }
    ports.tasks.take_over(id)?;
    let mut session = claim.session.clone();
    let closed = stop(id, &pane, &mut session, ports, pace);
    let run = Run {
        pane: Some(pane.clone()),
        session,
        outcome: Outcome::Ended(format!("closed by dash task close: {why}")),
    };
    ports.tasks.record(id, &run)?;
    let told = tell_list(id, &run.outcome, ports.list).and_then(|()| ports.tasks.told(id));
    // Its worktree stays for a reopen; its build output never holds work (run_task's own `clean`).
    let cleaned = ports.forge.clean(id);
    closed.map_err(|e| format!("{id}: its run ended, but its pane {pane} stayed open: {e}"))?;
    told.map_err(|e| format!("{id}: its run ended, but the list was not told yet: {e}"))?;
    cleaned.map_err(|e| format!("{id}: its run ended, but its build output stays: {e}"))?;
    Ok(match ours {
        true => format!("{id}: its run ended, its pane {pane} closed"),
        false => {
            format!("{id}: its run ended; pane {pane} lists no agent of it, so it stays as it is")
        }
    })
}

/// What `close` answers when no run of this store holds the task.
pub const NOT_HELD: &str = "no run of Rust's holds it";

/// The most of a deploy's word a row keeps: its sha or its error's start. The whole error is the flag's.
const DEPLOY_WORDS: usize = 300;

/// Tries at a prompt that did not take its Enter, a minute at dash's 2 s poll as session.sh gives a fresh session (its hooks
/// drop the first ones). Unbounded, one would answer a menu shown later.
const ENTERS: u32 = 30;

/// Whether `until` passed; else one poll's sleep.
fn past(until: Instant, poll: Duration) -> bool {
    let past = Instant::now() >= until;
    if !past {
        sleep(poll);
    }
    past
}

/// The session's pane up, `line` run in it, until its prompt shows.
fn start(line: &str, ports: &mut Ports, pace: Pace, pane: &str) -> Result<(), String> {
    let (terminal, agent) = (ports.terminal, ports.agent);
    let started = Instant::now();
    terminal.run(pane, line)?;
    loop {
        // A failed read is a look that saw nothing, and what it said names why at the limit.
        let shown = terminal.read(pane).unwrap_or_else(|e| e);
        match agent.screen(&shown) {
            Screen::Ready => break,
            Screen::Keys(keys) => keys.iter().try_for_each(|k| terminal.key(pane, k))?,
            Screen::Starting => {}
        }
        if past(started + pace.start_limit, pace.poll) {
            // What the pane shows names why: a launch that failed prints it there.
            let limit = pace.start_limit;
            let shown = shown.trim();
            return Err(format!(
                "no prompt within {limit:?}; the pane shows: {shown}"
            ));
        }
    }
    Ok(())
}

fn say(terminal: &dyn Terminal, pane: &str, words: &str) -> Result<(), String> {
    terminal.send(pane, words)?;
    terminal.key(pane, "Enter")
}

fn drive(
    id: &str,
    account: Option<&str>,
    attempts: u32,
    adopted: Option<String>,
    ports: &mut Ports,
    pace: Pace,
    left: &mut Left,
) -> Result<Outcome, String> {
    let (slot, session, before) = (&mut left.pane, &mut left.session, &mut left.before);
    let mut lap = Lap::new(ports.step);
    // An adopted session already holds its brief and its folder: none is read, nothing opened, started or told.
    let (folder, adopted) = match adopted {
        Some(folder) => (folder, true),
        None => (
            ports
                .forge
                .worktree(id)
                .map_err(|e| format!("worktree: {e}"))?,
            false,
        ),
    };
    lap.done("worktree");
    let brief = match adopted {
        true => String::new(),
        false => {
            let brief = ports.list.brief(id).map_err(|e| format!("brief: {e}"))?;
            match layer_of(ports, id) {
                Some(layer) => format!("{brief}\n\n{}", layer_line(&layer)),
                None => brief,
            }
        }
    };
    lap.done("brief");
    // A first try finds a PR only from before its run: a reopen resets the tries and may keep the branch (a dirty worktree `leave`
    // refused, a run the app's pass merged). A later try's may be its own, merged in the seconds before a crash. An open one that
    // needs its session (a hand-back) says what, first; one only behind main needs nothing of it: `land` updates it.
    let earlier = ports.forge.pr(id);
    let read = earlier.is_ok();
    let earlier = earlier.ok().flatten();
    let open = earlier.as_ref().filter(|pr| !pr.merged);
    let fix = open
        .filter(|_| !adopted)
        .and_then(|pr| match gate(ports.forge, pr) {
            Ok(Gate::Fix(words)) => Some(words),
            _ => None,
        });
    let prompt = match (attempts, &earlier) {
        (1, Some(pr)) if !pr.merged => format!(
            "This task was opened again: {} and this folder hold what an earlier run did.\n\n{brief}",
            pr.url
        ),
        (1, _) => brief,
        (n, _) => format!(
            "Try {n} at this task: the last run failed, and its worktree, this folder, holds what it did.\n\n{brief}"
        ),
    };
    // Before what the attempt says, never instead of it: a retry still learns its last run failed.
    let prompt = match fix {
        Some(fix) => format!("Its pull request needs this first: {fix}.\n\n{prompt}"),
        None => prompt,
    };
    let merged_before = earlier
        .filter(|pr| attempts == 1 && pr.merged)
        .map(|pr| pr.url);
    if read {
        *before = Some(merged_before.clone());
    }
    let id = id.to_string();
    let task = Task { id, folder, prompt };
    if !adopted {
        let opened = ports
            .terminal
            .open(&task.folder, &format!("rust {}", task.id));
        let opened = opened.map_err(|e| format!("open: {e}"))?;
        ports.tasks.opened(&task.id, &opened)?;
        // A page's word only: no run fails for it.
        if let Some(account) = account {
            let _ = ports.tasks.login(&task.id, account);
        }
        let pane = slot.insert(opened);
        lap.done("open");
        start(&ports.launcher.command(&task, account)?, ports, pace, pane)?;
        // Through `tell`: a screen that reads ready proves no agent, and a shell would run the brief as a command line.
        let polls = start_polls(pace);
        tell(&task.id, ports, pace, polls, pane, session, &task.prompt)?;
        lap.done("start");
    }
    let pane = slot.as_mut().expect("opened or adopted");
    let supervised = supervise(
        &task,
        account,
        ports,
        pace,
        pane,
        session,
        merged_before.as_deref(),
    );
    lap.done("supervise");
    let (outcome, gone) = supervised?;
    // Its session stopped to free its seat (`free_seat`): no pane left for the run to close.
    if gone {
        *slot = None;
    }
    Ok(outcome)
}

/// A start's listing has `start_limit`, as `Pace` says, not the few looks a session already known gets.
fn start_polls(pace: Pace) -> u32 {
    let polls = pace
        .start_limit
        .as_millis()
        .checked_div(pace.poll.as_millis());
    polls.map_or(pace.stop_polls, |n| n as u32)
}

/// This run's agent in its pane, the one way anything here reads a pane before acting on it: a terminal reuses pane ids, so a
/// listing that does not name this run's session is none of this run's. The first id listed is adopted and saved: nothing acts
/// on a pane before this run has opened it, and a crash after that still knows whose it is. An agent with no id names nobody, so
/// it reads as none: herdr lists a Claude's id at its prompt, before any words (measured 2026-11-06, within 2 s of its trust menu).
fn look(
    id: &str,
    pane: &str,
    session: &mut Option<String>,
    ports: &mut Ports,
) -> Result<Option<Seen>, String> {
    let Some(seen) = ports.terminal.agent(pane)? else {
        return Ok(None);
    };
    let Some(listed) = seen.session.as_deref() else {
        return Ok(None);
    };
    match session.as_deref() {
        Some(saved) if saved != listed => return Ok(None),
        Some(_) => {}
        None => {
            ports.tasks.adopted(id, listed)?;
            *session = Some(listed.to_string());
        }
    }
    Ok(Some(seen))
}

/// Words for a session that must be there to read them: its pane's shell would run them as a command line.
fn tell(
    id: &str,
    ports: &mut Ports,
    pace: Pace,
    polls: u32,
    pane: &str,
    session: &mut Option<String>,
    words: &str,
) -> Result<(), String> {
    for _ in 0..polls {
        if let Ok(Some(_)) = look(id, pane, session, ports) {
            return say(ports.terminal, pane, words);
        }
        sleep(pace.poll);
    }
    Err("the agent left before it could read what it was told".to_string())
}

/// What a session a limit stopped is told once its account has room.
const CONTINUE: &str = "continue";

/// What a session a dead network stopped is told once it is back, in `Dashboard.Network`'s @nudge's words.
const CARRY_ON: &str = "carry on: the network dropped and is back, resume where you stopped";

/// Until `ready`. A session that left ends the wait: the words that follow say so.
fn wait_until(
    mut ready: impl FnMut() -> bool,
    id: &str,
    ports: &mut Ports,
    pace: Pace,
    pane: &str,
    session: &mut Option<String>,
) {
    let mut gone = 0;
    while !ready() {
        gone = match look(id, pane, session, ports) {
            Ok(None) => gone + 1,
            _ => 0,
        };
        if gone >= pace.stop_polls {
            return;
        }
        sleep(pace.patient_poll);
    }
}

/// What a limit stop waits for, then does, ending with its session told to continue. He spends each login to 100% on purpose
/// (@nha, 2026-11-01: « use 100% of one sub that expires first before moving to the next »), so a session there moves to the next
/// login with room, the one whose week resets first (`resume`), rather than wait days on its own (six runs sat on second's week
/// from 11:25 on 2026-11-09, default at 4%; @nha, 2026-11-09: « fix the root cause not the surface one-time thing. »). Its own
/// login with room and its session there: told to continue in its pane. A run on no named login (no reading at its start) moves
/// to any login with room unless every one has room. A session gone for good moves to any login with room, its own too; with none
/// it waits, flagged once with the first reset. Readings that keep failing are flagged, once.
fn after_limit(
    task: &Task,
    account: &mut Option<String>,
    ports: &mut Ports,
    pace: Pace,
    pane: &mut String,
    session: &mut Option<String>,
) -> Result<(), String> {
    let (mut gone, mut unread, mut flagged) = (0, 0, false);
    loop {
        match ports.agent.limits() {
            Ok(limits) => {
                // A key's row is a headless run's, never this session's.
                let logins: Vec<&Limit> = limits.iter().filter(|l| !l.key).collect();
                let own = |l: &&&Limit| account.as_deref().is_none_or(|a| l.account == a);
                let own_room = logins.iter().filter(own).all(|l| l.room());
                if own_room && gone == 0 {
                    // Its limit menu still open would take the words.
                    close_menu(&task.id, ports, pace, pane, session);
                    // Gone in the looks `tell` takes: resumed as one gone for good, at the next reading.
                    let told = tell(
                        &task.id,
                        ports,
                        pace,
                        pace.stop_polls,
                        pane,
                        session,
                        CONTINUE,
                    );
                    if told.is_ok() {
                        return Ok(());
                    }
                    gone = pace.stop_polls;
                    continue;
                }
                let left = gone >= pace.stop_polls;
                // ponytail: the spend order `Defaults.pick` starts by, so a login never read (its reset 0) comes first.
                let mut room: Vec<&Limit> = logins.iter().copied().filter(|l| l.room()).collect();
                room.sort_by_key(|l| l.week_resets);
                let elsewhere =
                    |l: &&&Limit| left || account.as_deref().is_none_or(|a| l.account != a);
                let to = match room.iter().find(elsewhere) {
                    // Its own login back while a look missed it: a gap in the listing, looked at again.
                    _ if own_room && !left => None,
                    Some(l) => Some(Some(l.account.as_str())),
                    // No reading names a login: nothing says stop, so the launch picks one.
                    None => (left && logins.is_empty()).then_some(None),
                };
                if let Some(to) = to {
                    return resume(task, to, account, ports, pace, pane, session);
                }
                if left && !flagged {
                    // A launcher that resumes nothing (Codex's one login) fails the run now, never after days of waiting.
                    let id = session.as_deref().unwrap_or_default();
                    ports.launcher.resume(task, account.as_deref(), id, pane)?;
                    flagged = true;
                    let at = logins.iter().filter_map(|l| reset(l)).min();
                    let at = at.map_or(String::new(), |at| {
                        format!(", the first at {}", dash_core::stamp(at as i64 * 1000))
                    });
                    let what = format!(
                        "limited, its session gone, and no login has room: it resumes on the first that does{at}"
                    );
                    let _ = ports.list.flag(&task.id, &what);
                }
            }
            Err(e) => {
                unread += 1;
                if unread == JUDGE_ASKS {
                    let what = format!("held by a limit, its readings fail: {e}");
                    let _ = ports.list.flag(&task.id, &what);
                }
            }
        }
        gone = match look(&task.id, pane, session, ports) {
            Ok(None) => gone + 1,
            _ => 0,
        };
        sleep(pace.patient_poll);
    }
}

/// Whether the pane shows the agent's limit menu. A failed read is no menu.
fn is_menu(terminal: &dyn Terminal, agent: &dyn Agent, pane: &str) -> bool {
    terminal
        .read(pane)
        .is_ok_and(|shown| agent.limit_menu(&shown))
}

/// Claude's limit menu, closed: one Escape while its pane shows it, then looks until the session asks no more, so the words that
/// follow reach the prompt. Anything else it asks gets no key. A menu Escape left open shows again to `Watch`, as the same stop.
fn close_menu(id: &str, ports: &mut Ports, pace: Pace, pane: &str, session: &mut Option<String>) {
    let mut pressed = false;
    for _ in 0..pace.stop_polls {
        match look(id, pane, session, ports) {
            Ok(Some(seen)) if seen.asking && !pressed => {
                if !is_menu(ports.terminal, ports.agent, pane) {
                    return;
                }
                let _ = ports.terminal.key(pane, "Escape");
                pressed = true;
            }
            Ok(Some(seen)) if seen.asking => {}
            Ok(_) => return,
            Err(_) => {}
        }
        sleep(pace.poll);
    }
}

/// When a login spent has room again (Unix seconds): its full windows' last reset. None: room now, or no reset read.
fn reset(login: &Limit) -> Option<u64> {
    let five = (login.five_hours >= 100.0).then_some(login.five_hours_resets);
    let week = (login.week >= 100.0).then_some(login.week_resets);
    let at = five.into_iter().chain(week).max();
    at.filter(|at| *at > 0)
}

/// The session a limit stopped, moved to login `to` in the same worktree: its line built first, so a launcher that cannot
/// resume it leaves it as it is; its limit menu closed, the session told to leave and its pane closed (`stop`); then resumed by
/// its id in a new pane, its pane and login written at once as at a start, and told to continue. A pane that still lists a
/// session this run cannot name (a /clear gives Claude another id) may be this work: no resume starts beside it, as no retry
/// does in `run_task`. Flagged, so its card says where it went.
/// ponytail: Claude keeps a session's id on `--resume`; one that took a new id reads as gone, and the run fails into a retry.
fn resume(
    task: &Task,
    to: Option<&str>,
    account: &mut Option<String>,
    ports: &mut Ports,
    pace: Pace,
    pane: &mut String,
    session: &mut Option<String>,
) -> Result<(), String> {
    let id = session
        .clone()
        .ok_or("a limit stopped a session with no id: none to resume")?;
    let line = ports.launcher.resume(task, to, &id, pane)?;
    close_menu(&task.id, ports, pace, pane, session);
    stop(&task.id, pane, session, ports, pace)?;
    if ports.terminal.agent(pane)?.is_some() {
        return Err(format!(
            "limited, and its pane {pane} lists a session this run cannot name, so no resume starts beside it"
        ));
    }
    let opened = ports
        .terminal
        .open(&task.folder, &format!("rust {}", task.id));
    let opened = opened.map_err(|e| format!("open: {e}"))?;
    ports.tasks.opened(&task.id, &opened)?;
    *pane = opened;
    let from = std::mem::replace(account, to.map(String::from));
    if let Some(to) = to {
        let _ = ports.tasks.login(&task.id, to);
    }
    let (from, to) = (
        from.as_deref().unwrap_or("its login"),
        to.unwrap_or("the launch's pick"),
    );
    let what = format!("limited on {from}: resumed on {to}, the same session in a new pane {pane}");
    let _ = ports.list.flag(&task.id, &what);
    start(&line, ports, pace, pane)?;
    tell(
        &task.id,
        ports,
        pace,
        start_polls(pace),
        pane,
        session,
        CONTINUE,
    )
}

/// Until the API's host answers. An outage is an ordinary wait, but a probe that stays down may be the probe's fault (it knows no
/// proxy, and his network rewrites DNS), so it is flagged, once.
fn wait_for_network(
    id: &str,
    ports: &mut Ports,
    pace: Pace,
    pane: &str,
    session: &mut Option<String>,
) {
    let (agent, list) = (ports.agent, ports.list);
    let mut down = 0;
    let back = || {
        let up = agent.reachable();
        if !up {
            down += 1;
            if down == JUDGE_ASKS {
                let _ = list.flag(
                    id,
                    "waits on the network: its probe of the API keeps failing",
                );
            }
        }
        up
    };
    wait_until(back, id, ports, pace, pane, session);
}

/// The layer its task names, as the store's last sync of the list holds it: none when unread.
fn layer_of(ports: &Ports, id: &str) -> Option<String> {
    let queue = ports.tasks.queue().ok()?;
    queue.tasks.into_iter().find(|t| t.id == id)?.layer
}

/// The brief's line naming the layer, and what the judge holds (task rust-task-names-layer).
pub fn layer_line(layer: &str) -> String {
    format!(
        "Layer: this task changes {layer}. Change it there, in the design system ({}) for any markup, style or component: a \
         diff that draws a page outside it is held, whatever the report says.",
        dash_core::tasks::DESIGN_SYSTEM
    )
}

/// Looks with no answer before whoever coordinates hears of it.
const JUDGE_ASKS: u32 = 10;

/// The judge's word, however long it takes: an app restarting or a model out is no verdict, and a session waiting costs
/// nothing. A run never ends on it, as the older app's PM check judges the same stop at its next pass.
fn judged(task: &Task, said: &str, ports: &Ports, pace: Pace) -> Verdict {
    let mut asks = 0;
    loop {
        match ports.list.judge(&task.id, said) {
            Ok(Verdict::Met) => return held(task, ports),
            Ok(verdict) => return verdict,
            Err(e) => {
                asks += 1;
                if asks == JUDGE_ASKS {
                    let _ = ports
                        .list
                        .flag(&task.id, &format!("its judge gives no answer: {e}"));
                }
                sleep(pace.judge_poll);
            }
        }
    }
}

/// A met run whose task names a layer, held while its diff draws a page outside the design system. A diff that cannot be read
/// holds nothing, and whoever coordinates hears of it: the older app's PM check still judges the PR.
fn held(task: &Task, ports: &Ports) -> Verdict {
    let Some(layer) = layer_of(ports, &task.id) else {
        return Verdict::Met;
    };
    match ports.forge.diff(&task.id) {
        Ok(diff) => dash_core::tasks::held_by_layer(Verdict::Met, Some(&layer), &diff),
        Err(e) => {
            let _ = (ports.list).flag(
                &task.id,
                &format!("its diff, read for {layer}, failed: {e}"),
            );
            Verdict::Met
        }
    }
}

/// Words told again to a session that stops on the same thing, one per kind of stop: neither says anything of the other.
#[derive(Default)]
struct Slow {
    told: Option<Instant>,
    flagged: bool,
}

/// API errors back soon, the network up, before one is judged as any stop: the older app's `Dashboard.Network` tries a stop 3 times.
const SOON: u32 = 3;

impl Slow {
    /// How long to hold its words.
    fn rest(&self, retry: Duration) -> Option<Duration> {
        retry.checked_sub(self.told?.elapsed())
    }

    /// Whether it came back within a third of `retry`: Claude Code's own retries last minutes, so anything later followed real
    /// work and is a new stop.
    fn back_soon(&self, retry: Duration) -> bool {
        self.told.is_some_and(|told| told.elapsed() < retry / 3)
    }
}

/// Each stop judged until one lands. Never ends a session for its time: past the limit one is judged, and what is not met
/// goes on working.
fn supervise(
    task: &Task,
    account: Option<&str>,
    ports: &mut Ports,
    pace: Pace,
    pane: &mut String,
    session: &mut Option<String>,
    merged_before: Option<&str>,
) -> Result<(Outcome, bool), String> {
    let mut account = account.map(String::from);
    // Its session stopped by `land` to free its seat: returned with the outcome, so the run closes no pane.
    let mut gone = false;
    let mut nudges = 0;
    let mut watch = Watch::new();
    let (mut limit_slow, mut offline_slow) = (Slow::default(), Slow::default());
    // API errors in a row back soon after the words, the network up.
    let mut soon = 0;
    loop {
        let stop = watch.next(task, ports, pace, pane, session)?;
        let said = match &stop {
            // Nothing here answers it: whoever coordinates does. Flagged once a wait, as `patient` stops the next look; a flag
            // that did not arrive is tried again one wait on, or the question sits with nobody told.
            Stop::Asking(shown) => {
                match ports.list.flag(&task.id, shown) {
                    Ok(()) => watch.patient = true,
                    Err(_) => watch.waiting_since = None,
                }
                continue;
            }
            // No word of the session's: no judge, no nudge, none of its nudges spent. Until an API error keeps coming back with the
            // network up: nothing shows that waiting mends it, so it is judged as any stop, and its nudges bound it. One back later
            // followed real work, so it waits again.
            Stop::Turn(held @ (Outcome::Limited(_) | Outcome::Offline(_)))
                if matches!(held, Outcome::Limited(_))
                    || soon < SOON
                    || !offline_slow.back_soon(pace.limit_retry) =>
            {
                let offline = matches!(held, Outcome::Offline(_));
                let up = !offline || ports.agent.reachable();
                // What holds it is its limit or a dead network: an API error while the network answers is neither.
                let waits = !offline || !up;
                // A page's word only: neither a store nor an app that failed must end a wait.
                if waits {
                    let _ = ports.tasks.waiting(&task.id, Some(held));
                    let _ = ports.list.waiting(&task.id, Some(held));
                }
                // Back right after its words: a limit its readings do not show, an Opus one say, or an API error while the network
                // answers. Slowly, or it would speak every few seconds until a reset days away (`Dashboard.Sessions` learned the
                // same). A network still down needs no rest: its own wait holds the words.
                let slow = if offline {
                    &mut offline_slow
                } else {
                    &mut limit_slow
                };
                if offline {
                    let again = up && slow.back_soon(pace.limit_retry);
                    soon = if again { soon + 1 } else { 0 };
                }
                let hold = !offline || soon > 0;
                if let Some(rest) = slow.rest(pace.limit_retry).filter(|_| hold) {
                    if !slow.flagged {
                        slow.flagged = true;
                        let what = if offline {
                            "an API error while the network answers: it says carry on every so often"
                        } else {
                            "a limit its readings do not show: it says continue every so often"
                        };
                        let _ = ports.list.flag(&task.id, what);
                    }
                    sleep(rest);
                }
                if !offline {
                    after_limit(task, &mut account, ports, pace, pane, session)?;
                } else if !up {
                    wait_for_network(&task.id, ports, pace, pane, session);
                }
                if waits {
                    let _ = ports.tasks.waiting(&task.id, None);
                    let _ = ports.list.waiting(&task.id, None);
                }
                // Someone moved it meanwhile: as `Dashboard.Network`, a session at work gets no « carry on ».
                let moved = offline
                    && matches!(look(&task.id, pane, session, ports), Ok(Some(seen)) if !seen.idle);
                if offline && !moved {
                    tell(
                        &task.id,
                        ports,
                        pace,
                        pace.stop_polls,
                        pane,
                        session,
                        CARRY_ON,
                    )?;
                }
                let told = Instant::now();
                slow.told = Some(told);
                watch.took_words();
                // A wait of days is no time at work: else the first look reads past its limit and judges an old report. An API
                // error with the network up waited on nothing: its clock runs, so one that keeps coming back is judged at the limit.
                if waits {
                    watch.started = told;
                }
                continue;
            }
            Stop::Turn(outcome) => outcome.parts().1,
            Stop::Past => "",
        };
        let past_limit = stop == Stop::Past;
        let verdict = judged(task, said, ports, pace);
        let step = match ports.policy.after(&verdict, nudges, past_limit) {
            Step::Land => match land(task, ports, pace, merged_before, pane, session, &mut gone) {
                Landing::Merged(url) => return Ok((Outcome::Merged(url), gone)),
                Landing::Stuck(why) => return Ok((Outcome::Ended(why), gone)),
                // Its session left to free its seat: none to tell. Ended, its PR open, so `hand_backs` reopens the task and a new
                // run in the same worktree is told what to fix.
                Landing::Fix(why) if gone => {
                    let why = format!("met, its session ended while CI ran, then: {why}");
                    return Ok((Outcome::Ended(why), gone));
                }
                // Counted as any nudge: a merge that never takes must not cost turns for good.
                Landing::Fix(why) => {
                    let not_yet = Verdict::NotMet(why);
                    ports.policy.after(&not_yet, nudges, past_limit)
                }
            },
            step => step,
        };
        match step {
            Step::Nudge(words) => {
                nudges += 1;
                // A nudge is words too: an API error back soon after it stays judged, so the nudges keep their count.
                if matches!(stop, Stop::Turn(Outcome::Offline(_))) {
                    offline_slow.told = Some(Instant::now());
                }
                tell(
                    &task.id,
                    ports,
                    pace,
                    pace.stop_polls,
                    pane,
                    session,
                    &words,
                )?;
                watch.took_words();
            }
            Step::GiveUp(why) => return Ok((Outcome::Ended(why), gone)),
            // Past its time it still works, so what stops it later is still looked at. A stop kept waits on someone.
            Step::Keep | Step::Land => watch.patient |= !past_limit,
        }
    }
}

enum Landing {
    Merged(String),
    /// What the session must fix for it to land.
    Fix(String),
    /// Nothing moved within the limit: whoever coordinates looks, its PR and worktree kept.
    Stuck(String),
}

/// Failed merges before one reads as refused. A timeout looks the same, and may even have landed: the next look reads that.
const MERGES: u32 = 3;

/// What an open PR needs before it lands, read afresh.
enum Gate {
    Ready,
    /// No verdict yet: its CI runs.
    Wait,
    /// What its session must do first.
    Fix(String),
    /// Main moved under it, cleanly: `land` merges it in and waits on CI again.
    Behind,
}

/// Main first: GitHub runs no CI on a head that conflicts, and one behind main is built again once it holds it, whatever its
/// checks say now. Then its checks, which must be green on that head. Err: no verdict.
fn gate(forge: &dyn Forge, pr: &Pr) -> Result<Gate, String> {
    let url = &pr.url;
    match forge.base(pr)? {
        Base::Holds => {}
        Base::Behind => return Ok(Gate::Behind),
        Base::Conflicts(files) => {
            let files = match files.join(", ") {
                none if none.is_empty() => "files git did not name".to_string(),
                files => files,
            };
            return Ok(Gate::Fix(format!(
                "{url} conflicts with main in {files}. Merge the base branch into yours, resolve them and push"
            )));
        }
    }
    Ok(match forge.checks(pr)? {
        Checks::Green => Gate::Ready,
        Checks::Red(jobs) => Gate::Fix(format!("CI is red on {url}: {jobs}. Fix it and push")),
        Checks::Pending => Gate::Wait,
    })
}

/// A met task's PR, green, merged. Behind main, main is merged into its branch here and CI awaited on the new head, as often as
/// main moves, within `ci_limit`. Its session stays while a fix may be told to it; at the first wait on CI it is stopped and its
/// seat freed (`free_seat`), and this run drives the PR alone. What else stands in its way goes to its session while it is there.
/// Read afresh at each look: the session may push. One failed read is no verdict.
fn land(
    task: &Task,
    ports: &mut Ports,
    pace: Pace,
    merged_before: Option<&str>,
    pane: &str,
    session: &mut Option<String>,
    gone: &mut bool,
) -> Landing {
    let forge = ports.forge;
    let asked = Instant::now();
    let mut unread = String::new();
    let (mut failed_merges, mut failed_updates) = (0, 0);
    let mut updated = false;
    // One try at freeing its seat: a stop that failed keeps the session, as before, and is not run again at each poll.
    let mut tried = false;
    loop {
        // Only a wait on CI frees the seat: a failed read or merge is looked at again within a poll, its session still there.
        let mut ci_runs = false;
        let read = forge.pr(&task.id);
        // Read once after its update: a PR gone later is gone, said as any.
        let lagging = updated && matches!(read, Ok(None));
        updated &= lagging;
        match read {
            // Its update pushed, GitHub may still name the old head for a moment: no PR holds the local one until it reads it.
            Ok(None) if lagging => unread = " (its update not read yet)".into(),
            Ok(None) => {
                let branch = format!("task/{}", task.id);
                return Landing::Fix(format!(
                    "no pull request holds the head of {branch}. If one is open, its head is not yours: pull what was pushed or push what you have, never a second PR. If one merged holding all your work, say so and stop. Else push and open one as your brief says"
                ));
            }
            Ok(Some(pr)) if pr.merged && merged_before == Some(pr.url.as_str()) => {
                let url = &pr.url;
                return Landing::Fix(format!(
                    "{url} merged before this run: this task was opened again, and that PR holds none of its new work. Do the work, push, and open a new pull request"
                ));
            }
            Ok(Some(pr)) if pr.merged => return Landing::Merged(pr.url),
            Ok(Some(pr)) => match gate(forge, &pr) {
                Ok(Gate::Ready) => match forge.merge(&pr) {
                    Ok(()) => return Landing::Merged(pr.url),
                    Err(e) => {
                        failed_merges += 1;
                        if failed_merges >= MERGES {
                            // Never « merge the base branch »: main moving is this loop's to follow, not the session's.
                            let url = &pr.url;
                            return Landing::Fix(format!(
                                "the merge of {url} was refused: {e}. Find what holds it, fix that and push"
                            ));
                        }
                        unread = format!(" ({e})");
                    }
                },
                Ok(Gate::Fix(words)) => return Landing::Fix(words),
                // Once its CI settled: an update per move of main would start a build per move and land none.
                Ok(Gate::Behind) => match forge.checks(&pr) {
                    Ok(Checks::Pending) => ci_runs = true,
                    Ok(Checks::Red(jobs)) => {
                        let url = &pr.url;
                        return Landing::Fix(format!(
                            "CI is red on {url}: {jobs}. Fix it and push"
                        ));
                    }
                    Ok(Checks::Green) => match forge.update(&task.id, &pr) {
                        Ok(()) => (updated, ci_runs) = (true, true),
                        // Its own worktree may hold what stops it (a file it changed and did not commit): told once retries
                        // run out, never asked to merge main itself.
                        Err(e) => {
                            failed_updates += 1;
                            if failed_updates >= MERGES {
                                return Landing::Fix(format!(
                                    "{e}. Commit or drop what that names and say so: this run merges main in itself"
                                ));
                            }
                            unread = format!(" ({e})");
                        }
                    },
                    Err(e) => unread = format!(" ({e})"),
                },
                Ok(Gate::Wait) => ci_runs = true,
                Err(e) => unread = format!(" ({e})"),
            },
            Err(e) => unread = format!(" ({e})"),
        }
        if ci_runs && !tried {
            tried = true;
            *gone = free_seat(task, ports, pace, pane, session);
        }
        if past(asked + pace.ci_limit, pace.ci_poll) {
            let limit = pace.ci_limit;
            return Landing::Stuck(format!("no CI verdict within {limit:?}{unread}"));
        }
    }
}

/// A met run's session stopped and its pane closed, so it holds no seat while CI runs (four met runs held seats on 2026-11-09
/// waiting on the line cap's fix, which had none to start; task rust-met-frees-seat). Its claim stays this run's, its pane
/// dropped (`TaskStore::left`). Gone is what a look says after the stop, never the stop's word: a close that failed after the
/// session left leaves none to tell, and a session still listed keeps its seat, as before, the PR still landing. Flagged, so its
/// card says who drives the PR, and what did not go as meant. True: its session is gone.
fn free_seat(
    task: &Task,
    ports: &mut Ports,
    pace: Pace,
    pane: &str,
    session: &mut Option<String>,
) -> bool {
    let stopped = stop(&task.id, pane, session, ports, pace);
    if !matches!(look(&task.id, pane, session, ports), Ok(None)) {
        return false;
    }
    let alone = "its run drives the pull request alone";
    let what = match (stopped, ports.tasks.left(&task.id)) {
        (Ok(()), Ok(())) => {
            format!("met: its session ended to free its seat while CI runs; {alone}")
        }
        (Err(e), Ok(())) => format!(
            "met: its session ended to free its seat while CI runs, but its pane stayed open: {e}; {alone}"
        ),
        // The pane stays on the claim: its seat stays counted, the side within his seats.
        (_, Err(e)) => format!(
            "met: its session ended while CI runs, but its seat stays counted: {e}; {alone}"
        ),
    };
    let _ = ports.list.flag(&task.id, &what);
    true
}

#[derive(Debug, PartialEq)]
enum Stop {
    /// It ended a turn, saying this.
    Turn(Outcome),
    /// It waits on a question, showing this.
    Asking(String),
    /// Its time passed, and it still works.
    Past,
}

/// One session watched across its turns.
struct Watch {
    started: Instant,
    prompted: Instant,
    listed: bool,
    unlisted: u32,
    enters: u32,
    waiting_since: Option<Instant>,
    unread: String,
    /// It worked since the last words sent: a turn's end now is that turn's.
    took: bool,
    /// The turn already judged: the transcript still ends on it until the next words land.
    judged: Option<Outcome>,
    /// A stop already judged and kept, or a question flagged: nothing here will move it, so no limit ends the wait. Its next
    /// work ends that.
    patient: bool,
}

impl Watch {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            started: now,
            prompted: now,
            listed: false,
            unlisted: 0,
            enters: ENTERS,
            waiting_since: None,
            unread: String::new(),
            took: false,
            judged: None,
            patient: false,
        }
    }

    /// Words just went in: a new turn is due, so waiting is a fresh question.
    fn took_words(&mut self) {
        (self.took, self.patient) = (false, false);
        self.enters = ENTERS;
        self.waiting_since = None;
    }

    fn next(
        &mut self,
        task: &Task,
        ports: &mut Ports,
        pace: Pace,
        pane: &str,
        session: &mut Option<String>,
    ) -> Result<Stop, String> {
        let (terminal, agent) = (ports.terminal, ports.agent);
        loop {
            // One failed look is no verdict: a slow terminal must not end a healthy session, and the limit ends a silent one.
            if let Ok(seen) = look(&task.id, pane, session, ports) {
                // A terminal may list the agent some seconds after its prompt shows: gone means seen first.
                self.listed |= seen.is_some();
                let idle = seen.as_ref().is_some_and(|s| s.idle);
                let asking = seen.as_ref().is_some_and(|s| s.asking);
                if seen.is_some() && !idle && !asking {
                    self.took = true;
                    self.patient = false;
                }
                // Claude's limit menu (« Stop and wait for limit to reset ») asks, to the terminal: with its transcript's limit stop, it
                // is that stop, each time it shows, never a question (merge-gate's run sat on it as one, 2026-11-09). Read on a
                // question already flagged too, at its slow pace: a flag is no verdict that it is not the menu.
                let menu = asking && is_menu(terminal, agent, pane);
                if (idle || menu || seen.is_none())
                    && let Some(id) = session.as_deref()
                {
                    // The same for a transcript that failed to read: it names why at the limit.
                    match agent.outcome(&task.folder, id) {
                        Ok(Some(outcome))
                            if match menu {
                                true => matches!(outcome, Outcome::Limited(_)),
                                false => self.took || self.judged.as_ref() != Some(&outcome),
                            } =>
                        {
                            self.judged = Some(outcome.clone());
                            self.took = false;
                            self.patient = false;
                            self.waiting_since = None;
                            return Ok(Stop::Turn(outcome));
                        }
                        Ok(_) => {}
                        Err(e) => self.unread = format!(" ({e})"),
                    }
                }
                // Its supervisor restarts it in the same pane: one look with no agent is that gap.
                self.unlisted = if self.listed && seen.is_none() {
                    self.unlisted + 1
                } else {
                    0
                };
                if self.unlisted >= pace.stop_polls {
                    let unread = &self.unread;
                    return Err(format!("the agent left with no outcome{unread}"));
                }
                // Nothing will move it: a prompt never taken, a turn that ended without words, a question nobody here answers.
                let waiting = idle || asking;
                self.waiting_since =
                    waiting.then(|| self.waiting_since.unwrap_or_else(Instant::now));
                let stuck = self
                    .waiting_since
                    .is_some_and(|since: Instant| since.elapsed() >= pace.stuck_limit);
                if stuck && !self.patient {
                    let limit = pace.stuck_limit;
                    let shown = terminal.read(pane).unwrap_or_else(|e| e);
                    let shown = shown.trim();
                    if asking {
                        return Ok(Stop::Asking(format!(
                            "waiting {limit:?} on a question; the pane shows: {shown}"
                        )));
                    }
                    // Told to continue and still stopped there: its words never landed, or the stop came back too fast for a look
                    // to see it work. The same stop again, never a session stuck.
                    if let (false, Some(outcome @ (Outcome::Limited(_) | Outcome::Offline(_)))) =
                        (self.took, &self.judged)
                    {
                        return Ok(Stop::Turn(outcome.clone()));
                    }
                    let unread = &self.unread;
                    return Err(format!(
                        "waiting {limit:?} with no outcome{unread}; the pane shows: {shown}"
                    ));
                }
                if seen.as_ref().is_some_and(|s| !s.idle) {
                    self.enters = 0; // it took the prompt
                }
                if idle && self.enters > 0 {
                    // No reply yet: the first Enter may have gone to a menu still open; on an empty prompt it does nothing.
                    self.enters -= 1;
                    let _ = terminal.key(pane, "Enter");
                }
            }
            let (from, limit) = match self.listed {
                true => (self.started, pace.limit),
                false => (self.prompted, pace.start_limit),
            };
            // Its answer shows within a look, and an agent gone is still counted at the fast pace.
            let slow = self.patient && self.unlisted == 0;
            let poll = if slow { pace.patient_poll } else { pace.poll };
            if past(from + limit, poll) {
                if !self.listed {
                    let unread = &self.unread;
                    return Err(format!("no outcome within {limit:?}{unread}"));
                }
                // Judged, never closed for it (done-when 4): the next judging is one more limit away.
                self.started = Instant::now();
                return Ok(Stop::Past);
            }
        }
    }
}

/// This run's agent told to leave, so nothing supervising it reads an unasked death; then its pane closed in any case. A pane
/// that does not list it is left as it is: it may be another's by now.
/// ponytail: an agent that left by itself, or a start that never listed one, leaves its shell's workspace open, as nothing proves
/// the pane is this run's; a Claude up after its start limit sits there unbriefed, holding a seat, until he closes it;
/// a pane's start time from herdr, if those pile up.
fn stop(
    id: &str,
    pane: &str,
    session: &mut Option<String>,
    ports: &mut Ports,
    pace: Pace,
) -> Result<(), String> {
    let terminal = ports.terminal;
    // A failed look is no verdict: only a listing says whose this pane is.
    let mut here = Err("no look at it".to_string());
    for _ in 0..pace.stop_polls {
        here = look(id, pane, session, ports);
        // An agent with no id yet names itself within seconds: waited for, so it closes as this run's and a retry is not lost to it.
        let nameless = matches!(here, Ok(None))
            && matches!(terminal.agent(pane), Ok(Some(seen)) if seen.session.is_none());
        if here.is_ok() && !nameless {
            break;
        }
        sleep(pace.poll);
    }
    if here?.is_none() {
        return Ok(());
    }
    let _ = terminal.send(pane, &ports.agent.stop_words());
    for _ in 0..pace.stop_polls {
        match look(id, pane, session, ports) {
            Ok(None) => break,
            // A slash command's first Enter only closes its menu.
            Ok(Some(_)) => {
                let _ = terminal.key(pane, "Enter");
            }
            Err(_) => {}
        }
        sleep(pace.poll);
    }
    terminal.close(pane)
}
