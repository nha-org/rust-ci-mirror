//! Section 20b's orchestrator, through the ports alone. `next` frees what a crash left and picks; `run_task` carries one pick
//! from its claim to a merged PR: worktree, pane, the agent, its words, its turns judged, CI, the merge.
use dash_core::tasks::{
    Agent, Checks, Forge, Launcher, Outcome, Pick, Policy, Run, Screen, Snapshot, Step, Task,
    TaskList, TaskStore, Terminal, Verdict,
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
}

/// `start_limit` is for its prompt and for the terminal to list it after, `limit` the time after which a session still at work
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
}

/// What to do now: claims a crash left are freed first, their panes closed, one out of tries told about; then the policy picks
/// among what no run holds.
pub fn next(ports: &mut Ports) -> Result<Pick, String> {
    for freed in ports.tasks.free_crashed()? {
        if let Some(pane) = &freed.pane {
            // Gone already, or nothing to do about it: a retry opens its own.
            let _ = ports.terminal.close(pane);
        }
        if !ports.policy.retry(freed.attempts) {
            let (id, attempts) = (&freed.id, freed.attempts);
            let why = format!("its run crashed, {attempts} tries");
            let outcome = Outcome::Failed(why.clone());
            let (pane, session) = (freed.pane.clone(), None);
            let run = Run {
                pane,
                session,
                outcome,
            };
            ports.tasks.record(id, &run)?;
            if ports.list.ended(id, &why).is_ok() {
                ports.tasks.told(id)?;
            }
        }
    }
    let mut queue = ports.list.queue()?;
    let mut pickable = vec![];
    for task in queue.tasks {
        match ports.tasks.status(&task.id)?.as_deref() {
            None | Some("open") => pickable.push(task),
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
    queue.tasks = pickable;
    // A run claimed and not listed yet holds its seat: a start takes minutes, and each look would start one more. A failed
    // look counts it, the side that stays within his seats.
    let starting = ports.tasks.claims()?.into_iter().filter(|pane| {
        let listed = |p: &String| matches!(ports.terminal.agent(p), Ok(Some(_)));
        !pane.as_ref().is_some_and(listed)
    });
    let starting = starting.count();
    let sessions = ports.terminal.sessions()? + starting;
    // A failed read is no spend known: its Pick still holds to the seats.
    // ponytail: counts sessions, not the subagents Admission adds; a seat's worth over, only while one runs.
    let limits = ports.agent.limits().unwrap_or_default();
    Ok(ports.policy.pick(&Snapshot {
        queue,
        sessions,
        limits,
    }))
}

/// The list told how a finished run ended: a merge closes its task, anything else leaves its PR to the list's own pass.
fn tell_list(id: &str, outcome: &Outcome, list: &dyn TaskList) -> Result<(), String> {
    match outcome {
        Outcome::Merged(pr) => list.merged(id, pr),
        Outcome::Ended(why) | Outcome::Failed(why) => list.ended(id, why),
        Outcome::Done(_) | Outcome::Blocked(_) => Ok(()),
    }
}

/// One task's run, recorded whatever it ends as. Err: it was never claimed, or its record or the close failed.
pub fn run_task(
    id: &str,
    account: Option<&str>,
    ports: &mut Ports,
    pace: Pace,
) -> Result<Run, String> {
    let attempts = ports.tasks.claim(id)?;
    let (mut pane, mut session) = (None, None);
    let driven = drive(id, account, attempts, ports, pace, &mut pane, &mut session);
    let outcome = driven.unwrap_or_else(Outcome::Failed);
    let run = Run {
        pane,
        session,
        outcome,
    };
    // Recorded first: stop may take minutes on a hung terminal, and a run killed in them has still said how it ended.
    let recorded = match &run.outcome {
        Outcome::Failed(_) if ports.policy.retry(attempts) => ports.tasks.release(id, &run),
        _ => ports.tasks.record(id, &run),
    };
    let told = match &run.outcome {
        Outcome::Failed(_) if ports.policy.retry(attempts) => Ok(()),
        outcome => tell_list(id, outcome, ports.list).and_then(|()| ports.tasks.told(id)),
    };
    let closed = run.pane.as_deref().map_or(Ok(()), |p| stop(p, ports, pace));
    let left = match run.outcome {
        Outcome::Merged(_) => ports.forge.leave(id),
        _ => Ok(()),
    };
    recorded?;
    // `next` tells it again at each look.
    told.map_err(|e| format!("{id} is recorded, but the list was not told yet: {e}"))?;
    closed.map_err(|e| format!("{id} is recorded, but its pane stayed open: {e}"))?;
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
    say(terminal, pane, &task.prompt)
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
    let folder = ports
        .forge
        .worktree(id)
        .map_err(|e| format!("worktree: {e}"))?;
    let brief = ports.list.brief(id).map_err(|e| format!("brief: {e}"))?;
    let prompt = match attempts {
        1 => brief,
        n => format!(
            "Try {n} at this task: the last run failed, and its worktree, this folder, holds what it did.\n\n{brief}"
        ),
    };
    let id = id.to_string();
    let task = Task { id, folder, prompt };
    let opened = ports
        .terminal
        .open(&task.folder, &format!("rust {}", task.id));
    let opened = opened.map_err(|e| format!("open: {e}"))?;
    ports.tasks.opened(&task.id, &opened)?;
    let pane = pane.insert(opened).as_str();
    start(&task, account, ports, pace, pane)?;
    supervise(&task, ports, pace, pane, session)
}

/// Words for a session that must be there to read them: its pane's shell would run them as a command line.
fn tell(ports: &Ports, pace: Pace, pane: &str, words: &str) -> Result<(), String> {
    for _ in 0..pace.stop_polls {
        if let Ok(Some(_)) = ports.terminal.agent(pane) {
            return say(ports.terminal, pane, words);
        }
        sleep(pace.poll);
    }
    Err(format!("the agent left before it could read: {words}"))
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

/// Each stop judged until one lands. Never ends a session for its time: past the limit one is judged, and what is not met
/// goes on working.
fn supervise(
    task: &Task,
    ports: &Ports,
    pace: Pace,
    pane: &str,
    session: &mut Option<String>,
) -> Result<Outcome, String> {
    let mut nudges = 0;
    let mut watch = Watch::new();
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
            Stop::Turn(outcome) => outcome.parts().1,
            Stop::Past => "",
        };
        let past_limit = stop == Stop::Past;
        let verdict = judged(task, said, ports, pace);
        let step = match ports.policy.after(&verdict, nudges, past_limit) {
            Step::Land => match land(task, ports, pace) {
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
                tell(ports, pace, pane, &words)?;
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
fn land(task: &Task, ports: &Ports, pace: Pace) -> Landing {
    let forge = ports.forge;
    let asked = Instant::now();
    let mut unread = String::new();
    let mut failed_merges = 0;
    loop {
        match forge.pr(&task.id) {
            Ok(None) => {
                let branch = format!("task/{}", task.id);
                return Landing::Fix(format!(
                    "no pull request holds the head of {branch}: push it and open one as your brief says"
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
        ports: &Ports,
        pace: Pace,
        pane: &str,
        session: &mut Option<String>,
    ) -> Result<Stop, String> {
        let (terminal, agent) = (ports.terminal, ports.agent);
        loop {
            // One failed look is no verdict: a slow terminal must not end a healthy session, and the limit ends a silent one.
            if let Ok(seen) = terminal.agent(pane) {
                if let Some(id) = seen.as_ref().and_then(|s| s.session.clone()) {
                    *session = Some(id);
                }
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

/// The agent told to leave, so nothing supervising it reads an unasked death; then the pane closed in any case.
fn stop(pane: &str, ports: &Ports, pace: Pace) -> Result<(), String> {
    let terminal = ports.terminal;
    // A failed look is no verdict here either: only a listing without it says it left.
    let here = || !matches!(terminal.agent(pane), Ok(None));
    if here() {
        let _ = terminal.send(pane, &ports.agent.stop_words());
        for _ in 0..pace.stop_polls {
            // A slash command's first Enter only closes its menu.
            let _ = terminal.key(pane, "Enter");
            sleep(pace.poll);
            if !here() {
                break;
            }
        }
    }
    terminal.close(pane)
}
