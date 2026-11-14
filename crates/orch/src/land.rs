//! `dash land`'s core (task rust-stalled-port moved it out of the binary, which only shrinks: rust-binary-ratchet): wait for a
//! PR's checks, merge at the head they covered, close its task, deploy and save the monitors, each step a closure the binary's
//! `land_pr` hands in and a test stands in for. The stall watcher (`crate::stalled`) lands a green PR by running `dash land`
//! (`dash_forge::land`), so through it too.
use dash_core::tasks::{MONITORS_DATA, MergedGate, ORCHESTRATOR, TaskStore};
use std::time::Duration;

/// `dash land <pr>`: wait for its checks, merge at the checked head, close its task, deploy when it changed rust/, then save the
/// monitors when it changed their data: Datadog kept the old ones after PR 1499 until a hand ran `dash monitors save` (task
/// rust-deploy-saves-monitors). `changed` lists the PR's files; `save` runs the deployed binary's `dash monitors save`, and one
/// that fails is said and `flag`ged once, the merge and deploy standing. The steps are taken as closures so a test runs them
/// with stand-ins. Task rust-land-reads-head (PR 1367: land read the head before GitHub saw the push and its merge was
/// refused): the head is read again after each wait, through `reads`, and the merge names the head the wait covered. Task
/// rust-land-gates-merged: `gate` runs dash-gate's tests on that head merged with main first, and only a pass merges.
#[allow(clippy::too_many_arguments)]
pub fn land<W, G, M, C, D, E, V, F, S>(
    pr: &str,
    title: &str,
    reads: &mut dyn LandReads,
    mut wait: W,
    mut gate: G,
    mut merge: M,
    mut close: C,
    mut changed: D,
    mut deploy: E,
    mut save: V,
    mut flag: F,
    mut say: S,
) -> Result<Landed, (i32, String)>
where
    W: FnMut(&str) -> (i32, String, String),
    G: FnMut(&str) -> Result<MergedGate, String>,
    M: FnMut(&str, &str) -> Result<(), String>,
    C: FnMut(&str, &str, &str) -> Result<String, String>,
    D: FnMut(&str) -> Result<Vec<String>, String>,
    E: FnMut() -> Result<String, String>,
    V: FnMut() -> Result<String, String>,
    F: FnMut(&str),
    S: FnMut(&str),
{
    let mut head = settled_head(reads, &mut say)?;
    let mut moves = 0;
    loop {
        // a head the first check (land_held's) did not cover passes the review hold and parity before its wait
        reads
            .recheck(&head)
            .map_err(|e| (1, format!("land: refused: {e}")))?;
        say(&format!("land: waiting for checks on {pr} at {head}"));
        let (code, stdout, stderr) = wait(pr);
        if code == 1 {
            return Err((1, format!("land: checks red: {}", stderr.trim())));
        }
        if code != 0 {
            let why = if stderr.trim().is_empty() {
                stdout.trim()
            } else {
                stderr.trim()
            };
            return Err((code, format!("land: no verdict: {why}")));
        }
        let mut now = settled_head(reads, &mut say)?;
        if now == head {
            say(&format!("land: checks green on {pr} at {head}"));
            // #1578 and #1563 were green alone and red together: the tree GitHub's merge makes is tested here first.
            // ponytail: main may move between this run and the merge; a lock on main's tip if two land in the same minutes.
            match gate(&head) {
                Ok(MergedGate::Passed(words)) => say(&format!("land: {words}")),
                Ok(MergedGate::Red(words)) => return Ok(Landed::Red(words)),
                // GitHub's merge refuses a conflicting one and its refusal is read below; nothing is tested here, and none merges
                Ok(MergedGate::Conflicts(files)) => say(&format!(
                    "land: its merge with main conflicts here in {}: GitHub's merge decides",
                    files.join(", ")
                )),
                Err(e) => {
                    return Err((
                        3,
                        format!("land: no verdict: dash-gate not run on its merge with main: {e}"),
                    ));
                }
            }
            match merge(pr, &head) {
                Ok(()) => break,
                Err(e) if e.contains("cannot be cleanly created") => return conflicted(reads, &e),
                // pushed between the read and the merge: --match-head-commit refused it, the new head is waited on
                Err(e) if e.contains("Head branch was modified") => {
                    now = settled_head(reads, &mut say)?;
                    if now == head {
                        return Err((
                            3,
                            format!(
                                "land: no verdict: its merge was refused ({}), but its head still reads {head}: land it again",
                                e.trim()
                            ),
                        ));
                    }
                }
                Err(e) => return Err((1, format!("land: merge failed: {e}"))),
            }
        }
        moves += 1;
        if moves > LAND_MOVES {
            return Err((
                3,
                format!(
                    "land: no verdict: its head moved {moves} times during its waits, last to {now}: land it once its pushes stop"
                ),
            ));
        }
        say(&format!(
            "land: its head moved from {head} to {now}: waiting again on {now}, {head} never merged"
        ));
        head = now;
    }
    say(&format!("land: merged {pr}"));
    match task_id_from_title(title) {
        Some(id) => {
            // the merge is done: a close that fails (a title naming no task in the store) is said, never the deploy's end
            match close(&id, pr, title) {
                Ok(words) => say(&format!("land: closed {id}{words}")),
                Err(e) => say(&format!("land: {id} not closed: {}", e.trim())),
            }
        }
        None => say("land: no task id in title, not closing"),
    }
    // files not listed: both steps run, each harmless twice (a deploy holding main's rust/ skips its build, a save PUTs in place)
    let (rust, monitors) = match changed(pr) {
        Ok(files) => (
            files.iter().any(|path| path.starts_with("rust/")),
            files.iter().any(|path| path == MONITORS_DATA),
        ),
        Err(e) => {
            say(&format!(
                "land: its files not listed ({}), deploying and saving the monitors",
                e.trim()
            ));
            (true, true)
        }
    };
    if !rust {
        say("land: no rust/ change, not deploying");
        return Ok(Landed::Merged);
    }
    // the forge's deploy lock: a deploy already running is waited for, and said so
    say("land: deploying (rust/ changed), after any deploy already running");
    // the flag's words stay the same for the same fault, so `flagged` keeps one a day; the why is in land's output
    let unsaved = format!(
        "PR {pr} merged, but its monitors are not saved at Datadog: run dash monitors save"
    );
    let serves = match deploy() {
        Ok(serves) => serves,
        Err(e) if monitors => {
            flag(&unsaved);
            return Err((
                1,
                format!("land: deploy failed, the monitors not saved: {e}"),
            ));
        }
        Err(e) => return Err((1, format!("land: deploy failed: {e}"))),
    };
    say(&format!("land: deployed: {serves}"));
    if monitors {
        say(&format!(
            "land: saving the monitors ({MONITORS_DATA} changed)"
        ));
        match save() {
            Ok(saved) => say(&format!("land: monitors saved:\n{}", saved.trim_end())),
            Err(e) => {
                say(&format!("land: monitors not saved: {}", e.trim()));
                flag(&unsaved);
            }
        }
    }
    Ok(Landed::Merged)
}

/// How a land with green checks ended: merged, its merge refused as conflicting with main (GitHub's words), or red on its merge
/// with main (dash-gate's failing tests and the PRs it crosses), for `land_held` to hand back to its task once its own hold is gone.
#[derive(Debug, PartialEq)]
pub enum Landed {
    Merged,
    Conflicts(String),
    Red(String),
}

/// `dash land`'s hold around its land (done-when 1): the orchestrator's, refused while a run at work holds it; ended by the merge,
/// kept when it does not land, as the orchestrator still works on it.
/// Only another's live hold refuses it: a store that does not open, a word with no number (a branch name gh reads) or a write that
/// fails lands with no hold, said on stderr, as an urgent land is never blocked by its bookkeeping (/code-review of a76563791;
/// @nha, 2026-11-10: « urgent tasks may require that »).
pub fn land_held(
    tasks: Result<&mut dyn TaskStore, String>,
    pr: &str,
    parity: impl FnOnce() -> Result<(), String>,
    mut hand_back: impl FnMut(Option<&str>) -> Result<Option<String>, String>,
    land: impl FnOnce() -> Result<Landed, (i32, String)>,
) -> Result<(), (i32, String)> {
    // before the hold and the wait: a refused PR stays free for the run that ports its gaps, and needs no green run (task
    // rust-port-parity, /code-review)
    parity().map_err(|e| (1, format!("land: refused: {e}")))?;
    // GitHub runs no CI on a conflicting head, so the wait would end on no verdict: a finished run's task is queued again
    // instead, before this land's hold, which would hold its new run off (task rust-conflict-reopens-run)
    let conflicts = "land: not landed, it conflicts with main";
    match hand_back(None) {
        Ok(None) => {}
        Ok(Some(words)) => {
            return Err((
                1,
                format!("{conflicts}: its task is queued again, handed back to a new run: {words}"),
            ));
        }
        Err(e) => return Err((1, format!("{conflicts}, its task not queued again: {e}"))),
    }
    let held = tasks.and_then(|tasks| Ok((tasks, pr_number(pr)?)));
    let held = match held {
        Ok((tasks, n)) => {
            let refused = |e: &String| e.starts_with(&format!("#{n} is held by "));
            match tasks.hold_pr(n, ORCHESTRATOR, "dash land") {
                Err(e) if refused(&e) => return Err((1, format!("land: refused, {e}"))),
                Err(e) => Err(e),
                Ok(_) => Ok((tasks, n)),
            }
        }
        Err(e) => Err(e),
    };
    if let Err(e) = &held {
        eprintln!("land: no hold taken: {e}");
    }
    let landed = land();
    // a red land keeps its hold; one refused at a head that moved during its wait frees it, as the refusal above does
    let keeps = matches!(&landed, Err((_, e)) if !e.starts_with("land: refused"));
    // merged: a failed write leaves a hold that ends in 30 minutes. Refused as conflicting: the hold goes first, since
    // `hand_back_conflict` hands back no PR the orchestrator holds (task rust-land-reads-head, done-when 3)
    if let (Ok((tasks, n)), false) = (held, keeps) {
        let _ = tasks.unhold(n);
    }
    let (refused, red) = match landed? {
        Landed::Merged => return Ok(()),
        Landed::Conflicts(why) => (
            format!(
                "land: not landed, GitHub refused its merge ({why}) and reads it conflicting with main"
            ),
            None,
        ),
        // its words name the failing tests and the PRs it crosses, for the new run's first words
        Landed::Red(why) => (format!("land: not landed, {why}"), Some(why)),
    };
    Err((
        1,
        match hand_back(red.as_deref()) {
            Ok(Some(_)) if red.is_some() => {
                format!("{refused}: its task is queued again, handed back to a new run")
            }
            Ok(Some(words)) => {
                format!("{refused}: its task is queued again, handed back to a new run: {words}")
            }
            Ok(None) if red.is_some() => {
                format!("{refused}, its task not handed back: no finished run of its task holds it")
            }
            Ok(None) => format!(
                "{refused}, its task not handed back: no finished run of its task holds it, or the merge with main here sees no conflict"
            ),
            Err(e) => format!("{refused}, its task not queued again: {e}"),
        },
    ))
}

/// A PR's number from `dash land`'s or `dash pr`'s word: « 1390 », « #1390 » or its URL.
pub fn pr_number(pr: &str) -> Result<u64, String> {
    let n = match pr.split_once("/pull/") {
        Some((_, rest)) => rest.split('/').next().unwrap_or(rest),
        None => pr.trim_start_matches('#'),
    };
    n.parse().map_err(|_| format!("{pr}: no PR number"))
}

/// What `dash land` reads of its PR beyond the checks, and the pause between two reads; a test stands in for GitHub.
pub trait LandReads {
    /// The PR's head as GitHub's PR reads it, and its branch's as git reads it at origin (Err: not read).
    fn heads(&mut self) -> Result<(String, Result<String, String>), String>;
    /// GitHub's mergeable: MERGEABLE, CONFLICTING or UNKNOWN while it computes it.
    fn mergeable(&mut self) -> Result<String, String>;
    /// The review hold and parity at `head`, a no-op on the head they were last passed at.
    fn recheck(&mut self, head: &str) -> Result<(), String>;
    fn pause(&mut self);
}

/// Reads of a head or a mergeable that GitHub has not settled, `LAND_PAUSE` apart: 11 pauses, 55 s at most.
pub const LAND_TRIES: u32 = 12;
pub const LAND_PAUSE: Duration = Duration::from_secs(5);
/// Heads a land follows before it gives up on a branch still pushed to: each move is a new wait of up to `CI_DEADLINE`.
pub const LAND_MOVES: u32 = 5;

/// The PR's head once GitHub's PR shows its branch's (PR 1367: read before GitHub saw the push), waited for up to
/// `LAND_TRIES` reads and said. A branch not read is said and GitHub's head taken, as before this read existed.
pub fn settled_head(
    reads: &mut dyn LandReads,
    say: &mut dyn FnMut(&str),
) -> Result<String, (i32, String)> {
    let mut lag = String::new();
    for n in 1..=LAND_TRIES {
        // a read that fails is asked again, as a lag is: after a green wait of hours, one gh timeout never ends the land
        let (head, branch) = match reads.heads() {
            Ok(read) => read,
            Err(e) => {
                lag = format!("its head not read: {e}");
                if n < LAND_TRIES {
                    reads.pause();
                }
                continue;
            }
        };
        let branch = match branch {
            Ok(branch) => branch,
            Err(e) => {
                say(&format!(
                    "land: its branch not read ({e}), going on GitHub's head {head}"
                ));
                return Ok(head);
            }
        };
        if branch == head {
            return Ok(head);
        }
        if !lag.starts_with("GitHub's") {
            let bound = LAND_PAUSE.as_secs() * u64::from(LAND_TRIES - 1);
            say(&format!(
                "land: GitHub's PR shows head {head}, its branch is at {branch}: waiting up to {bound} s for GitHub to see the push"
            ));
        }
        lag = format!("GitHub's PR shows head {head}, its branch is at {branch}");
        if n < LAND_TRIES {
            reads.pause();
        }
    }
    Err((
        3,
        format!("land: no verdict: {lag}, still after {LAND_TRIES} reads: land it again"),
    ))
}

/// A merge GitHub refused as « cannot be cleanly created »: its mergeable read again first, UNKNOWN waited for up to
/// `LAND_TRIES` reads. CONFLICTING is handed back by `land_held`; anything else is no verdict, nothing handed back.
pub fn conflicted(reads: &mut dyn LandReads, refused: &str) -> Result<Landed, (i32, String)> {
    let refused = refused.lines().next().unwrap_or(refused).trim();
    let mut read = String::new();
    for n in 1..=LAND_TRIES {
        read = reads.mergeable().map_err(|e| {
            (
                1,
                format!("land: merge refused ({refused}), its mergeable not read: {e}"),
            )
        })?;
        if read != "UNKNOWN" {
            break;
        }
        if n < LAND_TRIES {
            reads.pause();
        }
    }
    match read.as_str() {
        "CONFLICTING" => Ok(Landed::Conflicts(refused.to_string())),
        _ => Err((
            3,
            format!(
                "land: no verdict: its merge was refused ({refused}), but GitHub reads it {read} now: land it again"
            ),
        )),
    }
}

/// The task id a PR title `[kind][<id>] ...` names, its second bracket.
pub fn task_id_from_title(title: &str) -> Option<String> {
    let after_first = title.split_once(']')?.1;
    let open = after_first.find('[')?;
    let close = after_first[open..].find(']')? + open;
    let id = &after_first[open + 1..close];
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}
