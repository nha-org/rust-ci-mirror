//! Adapter of section 3: the queue of heavy work and its executors (rfc-rust-unified sections 4 and 13). A job is a value; it
//! waits while the Mac is short of memory, runs on its executor, and its result comes back as field writes through the write
//! path (the core's `OpLog`), so a local run and a later remote one look the same to the views. Only the local executor is
//! built: delegation comes in a later version (@nha, 2026-11-04, on « some cloud delegation »: « yes, in a later version »).
use dash_core::{FieldWrite, Op, OpLog};
use std::collections::VecDeque;

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

#[cfg(test)]
mod tests {
    use super::*;
    use dash_core::{MemoryLog, Value};

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

    #[dash_meter::dash_test]
    #[should_panic(expected = "model « whisper » named twice")]
    fn a_model_named_twice_is_refused() {
        let _ = transcribe().model("whisper", |_| Ok(vec![]));
    }
}
