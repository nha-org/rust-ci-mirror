//! Section 20b's orchestrator, through the ports alone. `next` frees what a crash left and picks; `run_task` carries one pick
//! from its claim to a merged PR: worktree, pane, the agent, its words, its turns judged, CI, the merge.
use dash_core::tasks::{
    Agent, Checks, Claim, Forge, Launcher, Limit, Outcome, Pick, Policy, Queued, Run, Screen, Seen,
    Step, Task, TaskList, TaskStore, Terminal, Verdict, at_cap, plan,
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

/// The sessions a seat counts: those listed, and each run claimed and not listed yet, as a start takes minutes and each look
/// would start one more. A failed look counts it, the side that stays within his seats.
pub fn seats_taken(terminal: &dyn Terminal, claims: &[Claim]) -> Result<usize, String> {
    let listed = |claim: &&Claim| {
        let (Some(pane), Some(session)) = (&claim.pane, &claim.session) else {
            return false;
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
    for crashed in ports.tasks.crashed()? {
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
                Ok(Some(seen)) if seen.session == crashed.session => {
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
    let queue = ports.list.queue()?;
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
    let sessions = seats_taken(ports.terminal, &ports.tasks.claims()?)?;
    lap.done("sessions");
    // A failed read is no spend known: its Pick still holds to the seats.
    // ponytail: counts sessions, not the subagents Admission adds; a seat's worth over, only while one runs.
    let limits = ports.agent.limits().unwrap_or_default();
    lap.done("limits");
    // As a reopen held at the cap, and the app keeps one flag of the same words a day: nothing else says work stopped here.
    if let Some(full) = at_cap(&rows) {
        let new = |t: &&Queued| !rows.iter().any(|r| r.id == t.id);
        for task in queue.tasks.iter().filter(new) {
            let _ = ports.list.flag(&task.id, &full);
        }
    }
    let pick = plan(ports.policy, queue, &rows, sessions, limits).next;
    lap.done("pick");
    Ok(pick)
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
    let (mut pane, mut session) = (None, None);
    let driven = drive(id, account, attempts, ports, pace, &mut pane, &mut session);
    lap = Lap::new(lap.step);
    let outcome = driven.unwrap_or_else(Outcome::Failed);
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
    let left = match run.outcome {
        Outcome::Merged(_) => ports.forge.leave(id),
        _ => Ok(()),
    };
    lap.done("leave");
    recorded?;
    // `next` tells it again at each look.
    told.map_err(|e| format!("{id} is recorded, but the list was not told yet: {e}"))?;
    closed.map_err(|e| format!("{id}: its pane stayed open: {e}"))?;
    left.map_err(|e| format!("{id} merged, but its worktree stays: {e}"))?;
    Ok(run)
}

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

/// The session's pane up, its first words in. A retry says so: what the last run left is in the worktree.
fn start(
    task: &Task,
    account: Option<&str>,
    ports: &mut Ports,
    pace: Pace,
    pane: &str,
) -> Result<(), String> {
    let (terminal, agent) = (ports.terminal, ports.agent);
    let started = Instant::now();
    terminal.run(pane, &ports.launcher.command(task, account)?)?;
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
    ports: &mut Ports,
    pace: Pace,
    pane: &mut Option<String>,
    session: &mut Option<String>,
) -> Result<Outcome, String> {
    let mut lap = Lap::new(ports.step);
    let folder = ports
        .forge
        .worktree(id)
        .map_err(|e| format!("worktree: {e}"))?;
    lap.done("worktree");
    let brief = ports.list.brief(id).map_err(|e| format!("brief: {e}"))?;
    lap.done("brief");
    // A first try finds a PR only from before its run: a reopen resets the tries and may keep the branch (a dirty worktree `leave`
    // refused, a run the app's pass merged). A later try's may be its own, merged in the seconds before a crash.
    let earlier = (attempts == 1)
        .then(|| ports.forge.pr(id).ok().flatten())
        .flatten();
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
    let merged_before = earlier.filter(|pr| pr.merged).map(|pr| pr.url);
    let id = id.to_string();
    let task = Task { id, folder, prompt };
    let opened = ports
        .terminal
        .open(&task.folder, &format!("rust {}", task.id));
    let opened = opened.map_err(|e| format!("open: {e}"))?;
    ports.tasks.opened(&task.id, &opened)?;
    let pane = pane.insert(opened).as_str();
    lap.done("open");
    start(&task, account, ports, pace, pane)?;
    // Through `tell`: a screen that reads ready proves no agent, and a shell would run the brief as a command line.
    // A start's listing has `start_limit`, as `Pace` says, not the few looks a session already known gets.
    let polls = pace
        .start_limit
        .as_millis()
        .checked_div(pace.poll.as_millis());
    let polls = polls.map_or(pace.stop_polls, |n| n as u32);
    tell(&task.id, ports, pace, polls, pane, session, &task.prompt)?;
    lap.done("start");
    let outcome = supervise(
        &task,
        account,
        ports,
        pace,
        pane,
        session,
        merged_before.as_deref(),
    );
    lap.done("supervise");
    outcome
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

/// Until the account this run started on has room, or every account when it named none (no reading at its start). He spends each
/// account to 100% on purpose (@nha, 2026-11-01: « use 100% of one sub that expires first before moving to the next »), so this
/// is an ordinary wait: nobody is flagged, and a window past its reset reads as empty, so it ends by itself. Readings that keep
/// failing are flagged, once.
/// ponytail: a run on no named account waits on all of them; its session's own reading names its account, if that ever holds one.
fn wait_for_room(
    id: &str,
    account: Option<&str>,
    ports: &mut Ports,
    pace: Pace,
    pane: &str,
    session: &mut Option<String>,
) {
    let (agent, list) = (ports.agent, ports.list);
    let mut unread = 0;
    let room = || match agent.limits() {
        Ok(limits) => {
            // A key's row is a headless run's, never this session's.
            let its_own = |l: &&Limit| !l.key && account.is_none_or(|a| l.account == a);
            limits.iter().filter(its_own).all(Limit::room)
        }
        Err(e) => {
            unread += 1;
            if unread == JUDGE_ASKS {
                let _ = list.flag(id, &format!("held by a limit, its readings fail: {e}"));
            }
            false
        }
    };
    wait_until(room, id, ports, pace, pane, session);
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

/// Looks with no answer before whoever coordinates hears of it.
const JUDGE_ASKS: u32 = 10;

/// The judge's word, however long it takes: an app restarting or a model out is no verdict, and a session waiting costs
/// nothing. A run never ends on it, as the older app's PM check judges the same stop at its next pass.
fn judged(task: &Task, said: &str, ports: &Ports, pace: Pace) -> Verdict {
    let mut asks = 0;
    loop {
        match ports.list.judge(&task.id, said) {
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
    pane: &str,
    session: &mut Option<String>,
    merged_before: Option<&str>,
) -> Result<Outcome, String> {
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
                    wait_for_room(&task.id, account, ports, pace, pane, session);
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
                if !moved {
                    let words = if offline { CARRY_ON } else { CONTINUE };
                    tell(&task.id, ports, pace, pace.stop_polls, pane, session, words)?;
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
            Step::Land => match land(task, ports, pace, merged_before) {
                Landing::Merged(url) => return Ok(Outcome::Merged(url)),
                Landing::Stuck(why) => return Ok(Outcome::Ended(why)),
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
            Step::GiveUp(why) => return Ok(Outcome::Ended(why)),
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

fn refused(url: &str, why: &str) -> Landing {
    Landing::Fix(format!(
        "the merge of {url} was refused: {why}. Merge the base branch into yours and push"
    ))
}

/// A met task's PR, green, merged. What stands in its way goes to its session, which is still there. Read afresh at each look:
/// the session may push. One failed read is no verdict.
fn land(task: &Task, ports: &Ports, pace: Pace, merged_before: Option<&str>) -> Landing {
    let forge = ports.forge;
    let asked = Instant::now();
    let mut unread = String::new();
    let mut failed_merges = 0;
    loop {
        match forge.pr(&task.id) {
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
            Ok(Some(pr)) => match forge.checks(&pr) {
                Ok(Checks::Green) => match forge.stale(&pr) {
                    Ok(files) if files.is_empty() => match forge.merge(&pr) {
                        Ok(()) => return Landing::Merged(pr.url),
                        Err(e) => {
                            failed_merges += 1;
                            if failed_merges >= MERGES {
                                return refused(&pr.url, &e);
                            }
                            unread = format!(" ({e})");
                        }
                    },
                    Ok(files) => {
                        let files = files.join(", ");
                        let why = format!("the base branch changed {files} since its CI ran");
                        return refused(&pr.url, &why);
                    }
                    Err(e) => unread = format!(" ({e})"),
                },
                Ok(Checks::Red(jobs)) => {
                    let url = &pr.url;
                    return Landing::Fix(format!("CI is red on {url}: {jobs}. Fix it and push"));
                }
                Ok(Checks::Pending) => {}
                Err(e) => unread = format!(" ({e})"),
            },
            Err(e) => unread = format!(" ({e})"),
        }
        if past(asked + pace.ci_limit, pace.ci_poll) {
            let limit = pace.ci_limit;
            return Landing::Stuck(format!("no CI verdict within {limit:?}{unread}"));
        }
    }
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
                if (idle || seen.is_none())
                    && let Some(id) = session.as_deref()
                {
                    // The same for a transcript that failed to read: it names why at the limit.
                    match agent.outcome(&task.folder, id) {
                        Ok(Some(outcome))
                            if self.took || self.judged.as_ref() != Some(&outcome) =>
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
