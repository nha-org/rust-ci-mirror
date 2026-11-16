//! Extension of section 3: incremental views over the op log (section 6). Needs-you, the board and docs to review are one
//! differential dataflow on one timely worker: each commit's ops enter as one step, each view emits its inserts and
//! deletes, and a subscriber gets a view's rows, then each diff. `rules` holds what each row says and the plain recompute
//! the tests hold the dataflow to.
pub mod rules;

use dash_core::Op;
use differential_dataflow::VecCollection as Collection;
use differential_dataflow::input::{Input as _, InputSession};
use rules::*;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::Duration;
use timely::WorkerConfig;
use timely::communication::Allocator;
use timely::communication::allocator::thread::Thread;
use timely::dataflow::operators::probe::Handle;
use timely::worker::Worker;

type S = String;
type In<D> = InputSession<u64, D, isize>;
type Run = (S, (S, Option<S>, S, Vec<S>));
type Listed = (S, (S, Option<i64>, Vec<S>));
type Doc = (S, (DocFacts, Vec<(S, S)>, Option<S>));
/// One commit's changes to a view's rows: +1 a row in, -1 a row out.
pub type Diff = Vec<(Out, i64)>;

struct Inputs {
    runs: In<Run>,
    listed: In<Listed>,
    moved: In<(S, S)>,
    drafts: In<(S, S, S, S)>,
    sends: In<(S, S)>,
    docs: In<Doc>,
    switches: In<(S, S)>,
}

/// The dataflow and what it was fed. Not `Send`: timely's worker lives on one thread, `Views` gives it its own.
pub struct Engine {
    worker: Worker,
    inputs: Inputs,
    probe: Handle<u64>,
    sink: Rc<RefCell<Diff>>,
    time: u64,
    /// The cells of every row of the tables the views read: a field write retracts the row it changes.
    /// ponytail: a doc's body stays here as well as in the store, to decode its facts again on a stage write; facts kept per
    /// row instead once docs are many.
    cells: BTreeMap<(S, S), Cells>,
    /// Each view's rows with their weights, as the diffs left them.
    rows: BTreeMap<Out, i64>,
    subscribers: Vec<(S, Sender<Diff>)>,
}

/// `(id, ())`: a key alone, as `semijoin` and `antijoin` take it.
fn keys<'s>(c: Collection<'s, u64, S, isize>) -> Collection<'s, u64, (S, ()), isize> {
    c.map(|id| (id, ()))
}

impl Engine {
    pub fn new() -> Self {
        let alloc = Allocator::Thread(Thread::default());
        let config = WorkerConfig::default();
        let mut worker = Worker::new(config, alloc, Some(std::time::Instant::now()));
        let sink: Rc<RefCell<Diff>> = Rc::default();
        let tap = sink.clone();
        let (inputs, probe) = worker.dataflow::<u64, _, _>(move |scope| {
            let (h_runs, runs) = scope.new_collection::<Run, isize>();
            let (h_listed, listed) = scope.new_collection::<Listed, isize>();
            let (h_moved, moved) = scope.new_collection::<(S, S), isize>();
            let (h_drafts, drafts) = scope.new_collection::<(S, S, S, S), isize>();
            let (h_sends, sends) = scope.new_collection::<(S, S), isize>();
            let (h_docs, docs) = scope.new_collection::<Doc, isize>();
            let (h_switches, switches) = scope.new_collection::<(S, S), isize>();

            // the board: Rust's tasks not finished, each in its column
            let status = |want: &'static str| move |(_, (s, ..)): &Run| s == want;
            let finished = keys(runs.clone().filter(status("finished")).map(|(id, _)| id));
            let claimed = keys(runs.clone().filter(status("claimed")).map(|(id, _)| id));
            let merged = keys(
                runs.clone()
                    .filter(|(_, (s, o, ..))| s == "finished" && o.as_deref() == Some("merged"))
                    .map(|(id, _)| id),
            );
            let offered = keys(
                listed
                    .clone()
                    .filter(|(_, (_, pos, _))| pos.is_some())
                    .map(|(id, _)| id),
            );
            let waits =
                runs.clone()
                    .flat_map(|(id, (.., waits))| waits.into_iter().map(move |w| (w, id.clone())))
                    .concat(listed.clone().flat_map(|(id, (.., waits))| {
                        waits.into_iter().map(move |w| (w, id.clone()))
                    }))
                    .distinct();
            let unmet = keys(
                waits
                    .antijoin(merged.map(|(id, ())| id))
                    .map(|(_, id)| id)
                    .distinct(),
            );
            let shown = keys(
                offered
                    .clone()
                    .map(|(id, ())| id)
                    .concat(runs.clone().map(|(id, _)| id))
                    .distinct(),
            )
            .antijoin(finished.map(|(id, ())| id));
            let todo = shown
                .clone()
                .semijoin(offered.map(|(id, ())| id))
                .antijoin(claimed.clone().map(|(id, ())| id))
                .antijoin(unmet.map(|(id, ())| id));
            let in_progress = shown.clone().semijoin(claimed.clone().map(|(id, ())| id));
            // a card he moved, on the board and not claimed: his column wins over the computed one
            let moved = moved
                .semijoin(shown.clone().map(|(id, ())| id))
                .antijoin(claimed.clone().map(|(id, ())| id));
            let backlog = shown
                .antijoin(claimed.map(|(id, ())| id))
                .antijoin(todo.clone().map(|(id, ())| id));
            let columns = in_progress
                .map(|(id, ())| (id, column(true, false, false).to_string()))
                .concat(todo.map(|(id, ())| (id, column(false, true, false).to_string())))
                .concat(backlog.map(|(id, ())| (id, column(false, false, true).to_string())))
                .antijoin(moved.clone().map(|(id, _)| id))
                .concat(moved);
            let titles = listed.clone().map(|(id, (title, ..))| (id, title));
            let board = columns
                .clone()
                .join_map(titles, |id, col, title| board_row(id, col, title))
                .concat(
                    columns
                        .antijoin(listed.map(|(id, _)| id))
                        .map(|(id, col)| board_row(&id, &col, "")),
                );

            // docs to review, and needs-you: blocked runs, pending drafts, docs in review, their questions
            let reviewed = docs
                .clone()
                .filter(|(_, (facts, _, stage))| in_review(stage.as_deref(), facts.claimed));
            let docs_view = reviewed.clone().map(|(rel, (f, ..))| docs_row(&rel, &f));
            // the drafts view: each draft with its send's state, empty with none (/drafts follows it)
            let open = drafts.clone().map(|(id, _, _, record)| (id, record));
            let sent = sends.clone().map(|(id, _)| id).distinct();
            let drafts_view = (open.clone())
                .join_map(sends, |id, record, send| drafts_row(id, record, send))
                .concat(
                    open.antijoin(sent)
                        .map(|(id, record)| drafts_row(&id, &record, "")),
                );
            let needs_view = runs
                .filter(|(_, (s, o, ..))| is_blocked(s, o.as_deref()))
                .map(|(id, (.., said, _))| blocked_row(&id, &said))
                .concat(
                    drafts
                        .filter(|(_, status, ..)| status == "pending")
                        .map(|(id, _, to, _)| draft_row(&id, &to)),
                )
                .concat(reviewed.map(|(rel, (f, ..))| review_row(&rel, &f)))
                .concat(docs.flat_map(|(rel, (_, asked, _))| {
                    asked
                        .into_iter()
                        .map(move |(anchor, text)| question_row(&rel, &anchor, &text))
                }))
                // a set: two questions of one anchor and words are one row
                .distinct();

            let (probe, _) = board
                .concat(docs_view)
                .concat(needs_view)
                .concat(drafts_view)
                // the switches: each row as plugins.sqlite keeps it, what the menu and the off filter read
                .concat(switches.map(|(id, value)| switch_row(&id, &value)))
                .inspect(move |(out, _time, diff)| {
                    tap.borrow_mut().push((out.clone(), *diff as i64))
                })
                .probe();
            let inputs = Inputs {
                runs: h_runs,
                listed: h_listed,
                moved: h_moved,
                drafts: h_drafts,
                sends: h_sends,
                docs: h_docs,
                switches: h_switches,
            };
            (inputs, probe)
        });
        Self {
            worker,
            inputs,
            probe,
            sink,
            time: 0,
            cells: BTreeMap::new(),
            rows: BTreeMap::new(),
            subscribers: vec![],
        }
    }

    fn push(&mut self, row: Input, diff: isize) {
        let i = &mut self.inputs;
        match row {
            Input::Run {
                id,
                status,
                outcome,
                said,
                waits,
            } => i.runs.update((id, (status, outcome, said, waits)), diff),
            Input::Listed {
                id,
                title,
                pos,
                waits,
            } => i.listed.update((id, (title, pos, waits)), diff),
            Input::Moved { id, column } => i.moved.update((id, column), diff),
            Input::Send { id, state } => i.sends.update((id, state), diff),
            Input::Draft {
                id,
                status,
                receiver,
                record,
            } => i.drafts.update((id, status, receiver, record), diff),
            Input::Doc {
                rel,
                facts,
                questions,
                stage,
            } => i.docs.update((rel, (facts, questions, stage)), diff),
            Input::Switch { id, value } => i.switches.update((id, value), diff),
        }
    }

    /// One commit's ops as one step: what each view changed, consolidated. Ops on a table no view reads are skipped.
    pub fn commit(&mut self, ops: &[Op]) -> Diff {
        let mut touched: BTreeMap<(S, S), Option<Input>> = BTreeMap::new();
        for op in ops {
            let w = &op.write;
            // any op makes its row, as SQLite's upsert does; only the fields a view reads are kept
            let Some((_, fields)) = TABLES.iter().find(|(t, _)| *t == w.table) else {
                continue;
            };
            let key = (w.table.clone(), w.row.clone());
            // the row as it stood before this commit, once, so its retraction matches what was pushed; none before its first op
            if !touched.contains_key(&key) {
                let before = self
                    .cells
                    .get(&key)
                    .and_then(|c| decode(&w.table, &w.row, c));
                touched.insert(key.clone(), before);
            }
            let cells = self.cells.entry(key).or_default();
            if fields.contains(&w.field.as_str()) {
                cells.insert(w.field.clone(), w.value.clone());
            }
        }
        for ((table, row), before) in touched {
            let after = decode(&table, &row, &self.cells[&(table.clone(), row.clone())]);
            if before == after {
                continue;
            }
            if let Some(before) = before {
                self.push(before, -1);
            }
            if let Some(after) = after {
                self.push(after, 1);
            }
        }
        self.time += 1;
        let t = self.time;
        let i = &mut self.inputs;
        i.runs.advance_to(t);
        i.runs.flush();
        i.listed.advance_to(t);
        i.listed.flush();
        i.moved.advance_to(t);
        i.moved.flush();
        i.drafts.advance_to(t);
        i.drafts.flush();
        i.sends.advance_to(t);
        i.sends.flush();
        i.docs.advance_to(t);
        i.docs.flush();
        i.switches.advance_to(t);
        i.switches.flush();
        let probe = self.probe.clone();
        self.worker.step_while(|| probe.less_than(&t));
        // an inspect sees each row's diffs as they happen: consolidated here
        let mut changes = std::mem::take(&mut *self.sink.borrow_mut());
        changes.sort();
        let mut merged: Diff = vec![];
        for (out, diff) in changes {
            match merged.last_mut() {
                Some((last, weight)) if *last == out => *weight += diff,
                _ => merged.push((out, diff)),
            }
        }
        merged.retain(|(_, weight)| *weight != 0);
        for (out, diff) in &merged {
            let weight = self.rows.entry(out.clone()).or_default();
            *weight += diff;
            if *weight == 0 {
                self.rows.remove(out);
            }
        }
        self.subscribers.retain(|(view, tx)| {
            let mine: Diff = merged
                .iter()
                .filter(|(o, _)| o.0 == *view)
                .cloned()
                .collect();
            mine.is_empty() || tx.send(mine).is_ok()
        });
        merged
    }

    /// A view's rows, as the diffs so far left them.
    pub fn rows(&self, view: &str) -> Vec<Out> {
        let rows = self.rows.iter().filter(|(o, w)| o.0 == view && **w > 0);
        rows.map(|(o, _)| o.clone()).collect()
    }

    /// Every view's rows with their weights: one, unless a diff went wrong.
    pub fn weights(&self) -> &BTreeMap<Out, i64> {
        &self.rows
    }

    /// The input rows as the cells decode them now: what `recompute` reads.
    pub fn inputs(&self) -> Vec<Input> {
        let rows = self.cells.iter();
        rows.filter_map(|((t, r), c)| decode(t, r, c)).collect()
    }

    /// A view's rows at once as one diff of inserts, then each commit's diff to it.
    pub fn subscribe(&mut self, view: &str, tx: Sender<Diff>) {
        let now: Diff = self.rows(view).into_iter().map(|o| (o, 1)).collect();
        if tx.send(now).is_ok() {
            self.subscribers.push((view.into(), tx));
        }
    }

    /// The engine's rows against a plain recompute of the same cells: `None` when they agree.
    pub fn disagreement(&self) -> Option<String> {
        let inputs = self.inputs();
        disagreement(&self.rows, &recompute(inputs.iter()))
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

/// Where the views read the op log: the ops after a `(stamp, writer)`, at most a limit, empty once caught up.
pub type Source = Box<dyn FnMut(Option<(i64, &str)>, i64) -> Result<Vec<Op>, String> + Send>;

/// One log the views follow: its source, the most ops a read holds, and for a plugin's log its name: a failed read of it is
/// said and the log skipped until the next read, never the views' failure nor read as a log cut back.
pub struct Feed {
    pub read: Source,
    pub page: i64,
    pub plugin: Option<&'static str>,
}

/// How often the views read the log while someone subscribes: a subscriber hears of a write this late at most.
pub const FOLLOW: Duration = Duration::from_secs(1);

enum Ask {
    Rows(S, Sender<Result<Vec<Out>, String>>),
    Subscribe(S, Sender<Diff>),
}

/// The engine on its own thread, fed from each of `feeds` before each ask: a page reads a view caught up with every log.
/// Each store file has its own op log and clock (tasks, docs, drafts), so each feed keeps its own `dash_core::Cursor`. A log
/// replaced in place (a /settings restore) rebuilds the engine from every feed, each subscriber sent what changed (task views-cursor-replaced).
/// ponytail: the cursor is the last op read, so an op `Replica::apply` lands later with an older stamp is never read. Nothing
/// applies ops to a file the views follow today; the sync phase hands what it applies to the views when it does.
pub struct Views {
    tx: Mutex<Sender<Ask>>,
}

/// Each feed's ops since its cursor into `engine`; a feed whose log was replaced builds a new engine from every feed, its
/// subscribers moved over with the difference.
fn tail(
    engine: &mut Engine,
    feeds: &mut [Feed],
    cursors: &mut [dash_core::Cursor],
) -> Result<(), String> {
    let mut failed = vec![false; feeds.len()];
    // a plugin's log that does not read is said and skipped; any other failure is the views'
    let kept =
        |feed: &Feed, failed: &mut bool, read: Result<bool, String>| match (read, feed.plugin) {
            (Err(e), Some(name)) => {
                eprintln!("the views: the {name} did not read, tried again at the next read: {e}");
                *failed = true;
                Ok(false)
            }
            (read, _) => read,
        };
    let mut replaced = false;
    for ((feed, cursor), failed) in feeds.iter_mut().zip(cursors.iter_mut()).zip(&mut failed) {
        let read = cursor.replaced(&mut feed.read);
        replaced |= kept(feed, failed, read)?;
    }
    let old = replaced.then(|| {
        cursors.fill(dash_core::Cursor::default());
        std::mem::take(engine)
    });
    let mut fed = Ok(());
    for ((feed, cursor), failed) in feeds.iter_mut().zip(cursors.iter_mut()).zip(&mut failed) {
        if *failed {
            continue;
        }
        let read = cursor.tail(&mut feed.read, feed.page, |ops| {
            engine.commit(ops);
        });
        if let Err(e) = kept(feed, failed, read.map(|()| false)) {
            fed = Err(e);
            break;
        }
    }
    if let Some(old) = old {
        for (view, tx) in old.subscribers {
            let (was, now) = (old.rows.iter(), engine.rows.iter());
            let gone = was.filter(|(o, _)| o.0 == view && !engine.rows.contains_key(*o));
            let came = now.filter(|(o, _)| o.0 == view && !old.rows.contains_key(*o));
            let diff: Diff = (gone.map(|(o, _)| (o.clone(), -1)))
                .chain(came.map(|(o, _)| (o.clone(), 1)))
                .collect();
            if diff.is_empty() || tx.send(diff).is_ok() {
                engine.subscribers.push((view, tx));
            }
        }
    }
    fed
}

impl Views {
    pub fn start(mut feeds: Vec<Feed>) -> Result<Views, String> {
        let (tx, rx): (Sender<Ask>, Receiver<Ask>) = channel();
        std::thread::Builder::new()
            .name("views".into())
            .spawn(move || {
                let mut engine = Engine::new();
                let mut cursors = vec![dash_core::Cursor::default(); feeds.len()];
                // caught up at once, so the first page waits on no replay it could have been spared
                let _ = tail(&mut engine, &mut feeds, &mut cursors);
                loop {
                    // with a subscriber, the log is read every FOLLOW even when nobody asks; with none, only on an ask
                    let ask = if engine.subscribers.is_empty() {
                        rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
                    } else {
                        rx.recv_timeout(FOLLOW)
                    };
                    let ask = match ask {
                        Ok(ask) => Some(ask),
                        Err(RecvTimeoutError::Timeout) => None,
                        Err(RecvTimeoutError::Disconnected) => return,
                    };
                    // a read that fails on a follow tick is tried again on the next; an ask hears of it
                    let fed = tail(&mut engine, &mut feeds, &mut cursors);
                    match ask {
                        Some(Ask::Rows(view, reply)) => {
                            let _ = reply.send(fed.map(|()| engine.rows(&view)));
                        }
                        Some(Ask::Subscribe(view, sink)) => engine.subscribe(&view, sink),
                        None => {}
                    }
                }
            })
            .map_err(|e| format!("the views thread: {e}"))?;
        Ok(Views { tx: Mutex::new(tx) })
    }

    fn ask(&self, ask: Ask) -> Result<(), String> {
        let tx = self.tx.lock().map_err(|_| "the views' sender")?;
        tx.send(ask).map_err(|_| "the views thread ended".into())
    }

    /// A view's rows, once the views hold every op the log has.
    pub fn rows(&self, view: &str) -> Result<Vec<Out>, String> {
        let (reply, answer) = channel();
        self.ask(Ask::Rows(view.into(), reply))?;
        answer
            .recv()
            .map_err(|_| "the views thread ended".to_string())?
    }

    /// A view's rows, then each diff, as `Engine::subscribe`.
    pub fn subscribe(&self, view: &str) -> Result<Receiver<Diff>, String> {
        let (tx, rx) = channel();
        self.ask(Ask::Subscribe(view.into(), tx))?;
        Ok(rx)
    }
}

/// The core's views port, which a plugin reads a view through (task rust-orch-ports).
impl dash_core::View for Views {
    fn rows(&self, view: &str) -> Result<Vec<Out>, String> {
        Views::rows(self, view)
    }
}

#[cfg(test)]
mod tests;
