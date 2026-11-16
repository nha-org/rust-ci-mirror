//! Adapter of section 3: the queue of heavy work and its executors (rfc-rust-unified sections 4 and 13). A job is a value; it
//! waits while the Mac is short of memory, runs on its executor, and its result comes back as field writes through the write
//! path (the core's `OpLog`), so a local run and a later remote one look the same to the views. Only the local executor is
//! built: delegation comes in a later version (@nha, 2026-11-04, on « some cloud delegation »: « yes, in a later version »).
use dash_core::{FieldWrite, Host, Op, OpLog, Recurring};
use std::collections::VecDeque;
use std::time::Duration;

/// Section 13's value: its input's hash, the model, the executor (by name), and the free memory it needs to start.
#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    pub input: String,
    pub model: String,
    pub executor: String,
    pub need_mb: u64,
}

/// Where a job runs. The local one runs on the Mac; a delegate (Modal, a Fly Machine) is another impl, later.
pub trait Executor {
    fn name(&self) -> &str;
    /// The job's result as the rows it writes, never written here: the queue writes them.
    fn run(&self, job: &Job) -> Result<Vec<FieldWrite>, String>;
}

type Model = Box<dyn Fn(&Job) -> Result<Vec<FieldWrite>, String>>;

/// The Mac's executor: each model a function of the job, named as the job names it.
#[derive(Default)]
pub struct Local {
    models: Vec<(String, Model)>,
}

impl Local {
    pub fn model(
        mut self,
        name: &str,
        run: impl Fn(&Job) -> Result<Vec<FieldWrite>, String> + 'static,
    ) -> Self {
        assert!(
            self.models.iter().all(|(n, _)| n != name),
            "model « {name} » named twice"
        );
        self.models.push((name.into(), Box::new(run)));
        self
    }
}

impl Executor for Local {
    fn name(&self) -> &str {
        "local"
    }

    fn run(&self, job: &Job) -> Result<Vec<FieldWrite>, String> {
        let (_, run) = self
            .models
            .iter()
            .find(|(name, _)| *name == job.model)
            .ok_or_else(|| format!("no local model « {} »", job.model))?;
        run(job)
    }
}

/// What one tick did.
#[derive(Debug, PartialEq)]
pub enum Tick {
    Idle,
    /// The first job waits: free memory is under its need. Nothing behind it passes it.
    Waiting {
        need_mb: u64,
        free_mb: u64,
    },
    /// The first job ran; its result as the write path stamped it.
    Ran(Vec<Op>),
    /// The first job's run failed (no such executor or model, the model's error): it leaves the queue, so the next can run.
    Failed {
        job: Job,
        error: String,
    },
}

/// The jobs in order, and the executors they name.
pub struct Queue {
    jobs: VecDeque<Job>,
    executors: Vec<Box<dyn Executor>>,
    /// The first job's rows when the run finished but their write failed: the next tick writes them, never reruns the job.
    unwritten: Option<Vec<FieldWrite>>,
}

impl Queue {
    pub fn new(executors: Vec<Box<dyn Executor>>) -> Self {
        for (i, e) in executors.iter().enumerate() {
            let twice = executors[..i].iter().any(|o| o.name() == e.name());
            assert!(!twice, "executor « {} » named twice", e.name());
        }
        Self {
            jobs: VecDeque::new(),
            executors,
            unwritten: None,
        }
    }

    pub fn push(&mut self, job: Job) {
        self.jobs.push_back(job);
    }

    /// Runs the first job if `free_mb` covers its need, its rows written through `log`. `None`, memory that cannot be read,
    /// holds nothing back, as the orchestrator's start gate does. A failed run leaves the queue (`Failed`); a failed write
    /// keeps the rows for the next tick and is the error. The run happens on the caller's thread: a heavy job takes its own.
    // ponytail: strict order, so a big job holds the small ones behind it; let a fitting one pass if that ever starves them.
    // ponytail: reads memory, reserves none; a second heavy starter on the same reading can oversubscribe. One gate then.
    pub fn tick(&mut self, free_mb: Option<u64>, log: &mut dyn OpLog) -> Result<Tick, String> {
        if let Some(rows) = &self.unwritten {
            let ops = log.write(rows)?;
            self.unwritten = None;
            self.jobs.pop_front();
            return Ok(Tick::Ran(ops));
        }
        let Some(job) = self.jobs.front() else {
            return Ok(Tick::Idle);
        };
        if let Some(free_mb) = free_mb.filter(|free| *free < job.need_mb) {
            return Ok(Tick::Waiting {
                need_mb: job.need_mb,
                free_mb,
            });
        }
        let run = match self.executors.iter().find(|e| e.name() == job.executor) {
            Some(executor) => executor.run(job),
            None => Err(format!("no executor « {} »", job.executor)),
        };
        let rows = match run {
            Ok(rows) => rows,
            Err(error) => {
                let job = self.jobs.pop_front().expect("the first job");
                return Ok(Tick::Failed { job, error });
            }
        };
        let ops = log
            .write(&rows)
            .inspect_err(|_| self.unwritten = Some(rows.clone()))?;
        self.jobs.pop_front();
        Ok(Tick::Ran(ops))
    }
}

/// A recurring job as the loop holds it: its registration, its queue and its plugin's database once started, and when it is
/// due next.
struct Held {
    job: &'static Recurring,
    started: Option<(Queue, Box<dyn OpLog>)>,
    due_ms: i64,
    /// Its plugin's jobs off at its last due time, said then: said once a stop, not once a pass.
    off: bool,
}

/// Every plugin's `Recurring` jobs, each pushed on its own queue at its time and run there, its rows written to its plugin's
/// database (task rust-hostman-ci-wire). Never returns: the host gives it a thread of its own.
pub fn every(host: &'static dyn Host) {
    let mut held: Vec<Held> = inventory::iter::<Recurring>()
        .map(|job| Held {
            job,
            started: None,
            due_ms: 0,
            off: false,
        })
        .collect();
    loop {
        for said in pass(host, &mut held) {
            eprintln!("{said}");
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// One look at each job: one due starts if it has not, then runs once. What failed, said; a start that failed is tried again
/// at its next time, never dropped silently.
fn pass(host: &'static dyn Host, held: &mut [Held]) -> Vec<String> {
    let now = host.now_ms();
    let mut said = vec![];
    for h in held.iter_mut().filter(|h| now >= h.due_ms) {
        let job = h.job;
        // an off plugin's job waits, due, said once where its failures are (/code-review: rust/CLAUDE.md point 5, a stop said
        // where it is read), and runs at the next pass once it is on again (task rust-plugin-settings)
        match dash_core::registry::jobs_off(host, job.plugin) {
            Some(why) => {
                if !std::mem::replace(&mut h.off, true) {
                    said.push(format!("{}: {why}", job.model));
                }
                continue;
            }
            None => h.off = false,
        }
        h.due_ms = now + job.every_ms;
        if h.started.is_none() {
            let start = || -> Result<_, String> {
                let tick = (job.start)(host)?;
                let local = Local::default().model(job.model, move |_| Ok(tick()));
                let log = host.log(&host.data(), job.file, job.schema)?;
                Ok((Queue::new(vec![Box::new(local)]), log))
            };
            match start() {
                Ok(started) => h.started = Some(started),
                Err(e) => {
                    said.push(format!("{}: not started: {e}", job.model));
                    continue;
                }
            }
        }
        let Some((queue, log)) = h.started.as_mut() else {
            continue;
        };
        queue.push(Job {
            input: now.to_string(),
            model: job.model.into(),
            executor: "local".into(),
            need_mb: 0,
        });
        match queue.tick(None, &mut **log) {
            Ok(Tick::Failed { error, .. }) | Err(error) => {
                said.push(format!("{}: {error}", job.model))
            }
            Ok(_) => {}
        }
    }
    said
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_core::{MemoryLog, Value};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicI64, Ordering::Relaxed};

    fn transcribe() -> Local {
        Local::default().model("whisper", |job| {
            Ok(vec![FieldWrite {
                table: "transcripts".into(),
                row: job.input.clone(),
                field: "text".into(),
                value: Value::Text(format!("{} by {}", job.input, job.model)),
            }])
        })
    }

    fn job(need_mb: u64) -> Job {
        Job {
            input: "sha256:ab12".into(),
            model: "whisper".into(),
            executor: "local".into(),
            need_mb,
        }
    }

    #[dash_meter::dash_test]
    fn a_job_is_a_value_whose_result_comes_back_through_the_write_path() {
        let mut queue = Queue::new(vec![Box::new(transcribe())]);
        queue.push(job(0));
        let mut log = MemoryLog::default();
        let Tick::Ran(ops) = queue.tick(Some(1_000), &mut log).unwrap() else {
            panic!("it did not run")
        };
        assert_eq!(ops, log.0, "the rows returned are the ones the log stamped");
        assert_eq!(ops[0].write.table, "transcripts");
        assert_eq!(ops[0].write.row, "sha256:ab12");
        assert_eq!(
            ops[0].write.value,
            Value::Text("sha256:ab12 by whisper".into())
        );
        assert_eq!(
            queue.tick(Some(1_000), &mut log).unwrap(),
            Tick::Idle,
            "it ran once"
        );
    }

    #[dash_meter::dash_test]
    fn a_job_waits_while_free_memory_is_under_its_need() {
        let mut queue = Queue::new(vec![Box::new(transcribe())]);
        queue.push(job(4_000));
        let mut log = MemoryLog::default();
        for free_mb in [100, 3_999] {
            assert_eq!(
                queue.tick(Some(free_mb), &mut log).unwrap(),
                Tick::Waiting {
                    need_mb: 4_000,
                    free_mb
                }
            );
        }
        assert!(log.0.is_empty(), "nothing ran while short");
    }

    #[dash_meter::dash_test]
    fn a_waiting_job_starts_once_memory_frees() {
        let mut queue = Queue::new(vec![Box::new(transcribe())]);
        queue.push(job(4_000));
        let mut log = MemoryLog::default();
        assert!(matches!(
            queue.tick(Some(500), &mut log).unwrap(),
            Tick::Waiting { .. }
        ));
        assert!(
            matches!(queue.tick(Some(4_000), &mut log).unwrap(), Tick::Ran(ops) if ops.len() == 1)
        );
        assert_eq!(log.0.len(), 1, "it ran once memory freed");
        let mut unread = Queue::new(vec![Box::new(transcribe())]);
        unread.push(job(4_000));
        assert!(
            matches!(unread.tick(None, &mut log).unwrap(), Tick::Ran(_)),
            "an unread reading holds nothing back"
        );
    }

    #[dash_meter::dash_test]
    fn executor_is_a_trait_and_local_is_the_one_built() {
        let local: Box<dyn Executor> = Box::new(transcribe());
        assert_eq!(local.name(), "local");
        assert_eq!(local.run(&job(0)).unwrap().len(), 1);
        let unknown = Job {
            model: "nope".into(),
            ..job(0)
        };
        assert_eq!(local.run(&unknown).unwrap_err(), "no local model « nope »");
        let mut queue = Queue::new(vec![local]);
        queue.push(Job {
            executor: "modal".into(),
            ..job(0)
        });
        queue.push(job(0));
        let mut log = MemoryLog::default();
        let Tick::Failed { job: failed, error } = queue.tick(None, &mut log).unwrap() else {
            panic!("an unknown executor ran")
        };
        assert_eq!(
            error, "no executor « modal »",
            "no remote executor is built"
        );
        assert_eq!(failed.executor, "modal");
        assert!(
            matches!(queue.tick(None, &mut log).unwrap(), Tick::Ran(_)),
            "the failed job left, the next one ran"
        );
    }

    #[dash_meter::dash_test]
    fn a_failed_write_is_retried_without_running_the_job_again() {
        let runs = std::rc::Rc::new(std::cell::Cell::new(0));
        let counted = runs.clone();
        let local = Local::default().model("whisper", move |job| {
            counted.set(counted.get() + 1);
            transcribe().run(job)
        });
        let mut queue = Queue::new(vec![Box::new(local)]);
        queue.push(job(0));
        let mut log = MemoryLog(vec![], true);
        assert_eq!(queue.tick(None, &mut log).unwrap_err(), "disk full");
        log.1 = false;
        assert!(matches!(queue.tick(None, &mut log).unwrap(), Tick::Ran(ops) if ops.len() == 1));
        assert_eq!(runs.get(), 1, "the rows were kept, the job ran once");
        assert_eq!(queue.tick(None, &mut log).unwrap(), Tick::Idle);
    }

    type Rows = std::sync::Arc<std::sync::Mutex<Vec<FieldWrite>>>;

    /// A host whose clock is set by hand and whose database is one shared list of rows, refused while `closed`.
    #[derive(Default)]
    struct Clocked {
        now: AtomicI64,
        closed: AtomicBool,
        rows: Rows,
        jobs_off: AtomicBool,
    }

    impl dash_core::WallClock for Clocked {
        fn now_ms(&self) -> i64 {
            self.now.load(Relaxed)
        }
    }

    struct Shared(Rows);

    impl OpLog for Shared {
        fn write(&mut self, writes: &[FieldWrite]) -> Result<Vec<Op>, String> {
            self.0.lock().unwrap().extend_from_slice(writes);
            MemoryLog::default().write(writes)
        }
        fn apply(&mut self, _: &[Op]) -> Result<usize, String> {
            Ok(0)
        }
        fn ops_after(&self, _: Option<(i64, &str)>, _: i64) -> Result<Vec<Op>, String> {
            Ok(vec![])
        }
        fn state_hash(&self) -> Result<u64, String> {
            Ok(0)
        }
    }

    /// The switches' database the host keeps: every-test's jobs off while it holds true.
    struct Switched(bool);

    impl OpLog for Switched {
        fn write(&mut self, _: &[FieldWrite]) -> Result<Vec<Op>, String> {
            Err("read only".into())
        }
        fn apply(&mut self, _: &[Op]) -> Result<usize, String> {
            Ok(0)
        }
        fn ops_after(&self, _: Option<(i64, &str)>, _: i64) -> Result<Vec<Op>, String> {
            let write = FieldWrite {
                table: "switch".into(),
                row: "on/every-test/job".into(),
                field: "value".into(),
                value: Value::Text("off".into()),
            };
            Ok(self
                .0
                .then(|| Op {
                    stamp: 1,
                    writer: "t".into(),
                    write,
                })
                .into_iter()
                .collect())
        }
        fn state_hash(&self) -> Result<u64, String> {
            Ok(0)
        }
    }

    impl Host for Clocked {
        fn repo(&self) -> PathBuf {
            "/repo".into()
        }
        fn data(&self) -> PathBuf {
            "/data".into()
        }
        fn switched(&self) -> Option<PathBuf> {
            Some("/data".into())
        }
        fn home(&self) -> Result<PathBuf, String> {
            Err("none".into())
        }
        fn arm(&self, _: &Path, _: &str) {}
        fn log(&self, _: &Path, file: &str, _: &[&str]) -> Result<Box<dyn OpLog>, String> {
            if file == dash_core::switches::FILE {
                return Ok(Box::new(Switched(self.jobs_off.load(Relaxed))));
            }
            assert_eq!(file, "every-test.sqlite");
            match self.closed.load(Relaxed) {
                true => Err("disk gone".into()),
                false => Ok(Box::new(Shared(self.rows.clone()))),
            }
        }
    }

    inventory::submit! {
        Recurring { model: "every-test", plugin: "every-test", every_ms: 60_000, file: "every-test.sqlite", schema: &[], start: |host| {
            Ok(Box::new(move || vec![FieldWrite {
                table: "beats".into(),
                row: "one".into(),
                field: "at".into(),
                value: Value::Int(host.now_ms()),
            }]))
        } }
    }

    /// Done-when 1 of rust-hostman-ci-wire: a plugin's registered job runs on its queue at its time, once a period, its rows in
    /// its own database; a start that fails is said and tried again at the next time.
    #[dash_meter::dash_test]
    fn a_registered_job_runs_once_a_period_and_a_failed_start_is_said_and_retried() {
        let host: &'static Clocked = Box::leak(Box::default());
        let mut held = every_held();
        host.closed.store(true, Relaxed);
        assert_eq!(
            pass(host, &mut held),
            ["every-test: not started: disk gone"]
        );
        host.closed.store(false, Relaxed);
        host.now.store(30_000, Relaxed);
        assert!(pass(host, &mut held).is_empty(), "not due yet");
        assert!(host.rows.lock().unwrap().is_empty());
        for ms in [60_000, 90_000, 120_000] {
            host.now.store(ms, Relaxed);
            assert!(pass(host, &mut held).is_empty());
        }
        let ran: Vec<Value> = (host.rows.lock().unwrap().iter())
            .map(|r| r.value.clone())
            .collect();
        assert_eq!(
            ran,
            [Value::Int(60_000), Value::Int(120_000)],
            "once a period, from the retried start"
        );
    }

    fn every_held() -> Vec<Held> {
        (inventory::iter::<Recurring>.into_iter())
            .map(|job| Held {
                job,
                started: None,
                due_ms: 0,
                off: false,
            })
            .collect()
    }

    /// Done-when 2 of rust-plugin-settings: an off plugin's registered job neither starts nor runs; turned on again, it runs at
    /// the next pass.
    #[dash_meter::dash_test]
    fn an_off_plugin_s_job_waits_and_runs_once_on_again() {
        let host: &'static Clocked = Box::leak(Box::default());
        host.jobs_off.store(true, Relaxed);
        let mut held = every_held();
        let off = "every-test: off: the every-test plugin's jobs are off on /settings";
        for (ms, said) in [(0, vec![off]), (60_000, vec![])] {
            host.now.store(ms, Relaxed);
            assert_eq!(pass(host, &mut held), said, "said once a stop");
        }
        assert!(host.rows.lock().unwrap().is_empty(), "off: it never ran");
        host.jobs_off.store(false, Relaxed);
        // past FRESH_MS, so the switches are read again
        host.now.store(70_000, Relaxed);
        assert!(pass(host, &mut held).is_empty());
        let ran: Vec<Value> = (host.rows.lock().unwrap().iter())
            .map(|r| r.value.clone())
            .collect();
        assert_eq!(ran, [Value::Int(70_000)], "on again: it ran at once");
    }

    #[dash_meter::dash_test]
    #[should_panic(expected = "model « whisper » named twice")]
    fn a_model_named_twice_is_refused() {
        let _ = transcribe().model("whisper", |_| Ok(vec![]));
    }

    #[dash_meter::dash_test]
    #[should_panic(expected = "executor « local » named twice")]
    fn an_executor_named_twice_is_refused() {
        let _ = Queue::new(vec![Box::new(transcribe()), Box::new(transcribe())]);
    }
}
