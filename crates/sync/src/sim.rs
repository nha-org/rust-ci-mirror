//! Section 17's simulation, after nanolathe's headless runs: one thread, simulated time, one random stream, so a seed is a
//! whole run. Two nodes write and exchange ops over a network that delays, loses and reorders; a hash per event is the
//! trace, as a tick's hash is there. The adapters of the core's clock, randomness and scheduler ports are here.
use crate::{Msg, Node};
use dash_core::{FieldWrite, Fnv, Op, OpLog, Rng, Scheduler, Value, WallClock};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

const START_MS: i64 = 1_000_000;
const WRITES: u64 = 40;
const WORK_MS: u64 = 4_000;
const SYNC_MS: u64 = 100;
const MAX_SKEW_MS: u64 = 500;
/// Far past what a healthy run takes, so a run that cannot settle ends and says so.
const DEADLINE_MS: i64 = 120_000;

/// SplitMix64 (Steele et al., 2014). A change to it changes every seed's run.
pub struct SplitMix(u64);

impl SplitMix {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }
}

impl Rng for SplitMix {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let z = (self.0 ^ (self.0 >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        let z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

/// A node's view of simulated time: the loop's time, plus this node's own skew. Atomic only because a Replica is Send.
pub struct SimClock {
    now: Arc<AtomicI64>,
    skew_ms: i64,
}

impl WallClock for SimClock {
    fn now_ms(&self) -> i64 {
        self.now.load(Ordering::Relaxed) + self.skew_ms
    }
}

/// Earliest due first, then first scheduled.
pub struct Queue<E> {
    events: BTreeMap<(i64, u64), E>,
    scheduled: u64,
}

impl<E> Default for Queue<E> {
    fn default() -> Self {
        Self {
            events: BTreeMap::new(),
            scheduled: 0,
        }
    }
}

impl<E> Scheduler<E> for Queue<E> {
    fn at(&mut self, due_ms: i64, event: E) {
        self.events.insert((due_ms, self.scheduled), event);
        self.scheduled += 1;
    }

    fn pop(&mut self) -> Option<(i64, E)> {
        self.events.pop_first().map(|((due, _), e)| (due, e))
    }
}

#[derive(Clone, Copy)]
pub struct Faults {
    pub loss_pct: u64,
    /// Each message takes 1 ms to this, drawn alone, so a later send can overtake an earlier one.
    pub max_delay_ms: u64,
    /// A planted bug: a lost push is acked anyway, as a sender that trusts its send would act. No correct run survives it,
    /// so it proves the failure path end to end.
    pub lying_ack: bool,
}

impl Faults {
    /// One message in five lost; delays twice the sync period, so sends overtake each other.
    pub const ROUGH: Self = Self {
        loss_pct: 20,
        max_delay_ms: 2 * SYNC_MS,
        lying_ack: false,
    };
}

/// Makes a node's log, reading time from the clock it is given.
pub type MakeLog = dyn Fn(&str, SimClock) -> Result<Box<dyn OpLog>, String>;

pub enum Event {
    Write(usize),
    Timer(usize),
    Deliver { to: usize, nth: u64, msg: Msg },
}

#[derive(Default, Clone, Copy, Debug, PartialEq)]
pub struct Stats {
    pub sent: u64,
    pub lost: u64,
    pub reordered: u64,
}

#[derive(Debug, PartialEq)]
pub struct Outcome {
    /// Both nodes acked all, before the deadline.
    pub settled: bool,
    pub hashes: [u64; 2],
    /// One hash per event: the event's time and kind and the touched node's state.
    pub trace: Vec<u64>,
    pub logs: [Vec<Op>; 2],
    pub stats: Stats,
}

impl Outcome {
    pub fn one_state(&self) -> bool {
        self.settled && self.hashes[0] == self.hashes[1]
    }

    /// The whole trace as one number: two runs share it only if they went alike, event by event.
    pub fn fingerprint(&self) -> u64 {
        let mut hash = Fnv::new();
        for step in &self.trace {
            hash.bytes(&step.to_le_bytes());
        }
        hash.finish()
    }
}

/// 1 to 3 fields on a few rows of `notes (id, title, body)`, so writes collide. Any bits make a float, NaN included.
fn writes(rng: &mut dyn Rng) -> Vec<FieldWrite> {
    (0..=rng.below(3))
        .map(|_| {
            let (row, field, kind, n) = (rng.below(4), rng.below(2), rng.below(5), rng.next_u64());
            FieldWrite {
                table: "notes".into(),
                row: format!("n{row}"),
                field: ["title", "body"][field as usize].into(),
                value: match kind {
                    0 => Value::Null,
                    1 => Value::Int(n as i64),
                    2 => Value::Real(f64::from_bits(n)),
                    3 => Value::Text(format!("{n:x}")),
                    _ => Value::Blob(n.to_le_bytes().to_vec()),
                },
            }
        })
        .collect()
}

struct Net<'a> {
    rng: &'a mut dyn Rng,
    queue: &'a mut dyn Scheduler<Event>,
    faults: Faults,
    stats: Stats,
    sent: [u64; 2],
    highest: [u64; 2],
}

impl Net<'_> {
    fn send(&mut self, from: usize, now: i64, msg: Msg) {
        // Both draws always, so a fault's outcome never shifts the stream under what follows (nanolathe's I4).
        let lost = self.rng.below(100) < self.faults.loss_pct;
        let due = now + 1 + self.rng.below(self.faults.max_delay_ms.max(1)) as i64;
        self.stats.sent += 1;
        self.sent[1 - from] += 1;
        let nth = self.sent[1 - from];
        if !lost {
            self.queue.at(
                due,
                Event::Deliver {
                    to: 1 - from,
                    nth,
                    msg,
                },
            );
            return;
        }
        self.stats.lost += 1;
        if let (true, Msg::Push(ops)) = (self.faults.lying_ack, &msg)
            && let Some(op) = ops.last()
        {
            let ack = Msg::Ack(op.stamp, op.writer.clone());
            self.queue.at(
                due,
                Event::Deliver {
                    to: from,
                    nth: 0,
                    msg: ack,
                },
            );
        }
    }

    /// Whether this delivery came after one sent later on its way.
    fn arrived(&mut self, to: usize, nth: u64) {
        if nth < self.highest[to] && nth > 0 {
            self.stats.reordered += 1;
        }
        self.highest[to] = self.highest[to].max(nth);
    }
}

/// One whole run, every choice drawn from `rng`, every event through `queue`, every time from the clock it makes: nothing
/// else may decide (crates/gate holds this crate to it). A protocol error ends the run; the caller names the seed.
pub fn run(
    rng: &mut dyn Rng,
    queue: &mut dyn Scheduler<Event>,
    faults: Faults,
    make_log: &MakeLog,
) -> Result<Outcome, String> {
    let now = Arc::new(AtomicI64::new(START_MS));
    let mut node = |writer: &str| -> Result<Node, String> {
        let skew_ms = rng.below(MAX_SKEW_MS) as i64;
        let now = now.clone();
        Ok(Node::new(make_log(writer, SimClock { now, skew_ms })?))
    };
    let mut nodes = [node("a")?, node("b")?];
    for _ in 0..WRITES {
        let (due, on) = (rng.below(WORK_MS), rng.below(2));
        queue.at(START_MS + due as i64, Event::Write(on as usize));
    }
    for on in 0..2 {
        queue.at(START_MS + rng.below(SYNC_MS) as i64, Event::Timer(on));
    }
    let mut net = Net {
        rng,
        queue,
        faults,
        stats: Stats::default(),
        sent: [0; 2],
        highest: [0; 2],
    };
    let (mut written, mut settled, mut trace) = (0, false, vec![]);
    let mut hashes = [nodes[0].log().state_hash()?, nodes[1].log().state_hash()?];
    while let Some((due, event)) = net.queue.pop() {
        if due > START_MS + DEADLINE_MS {
            break;
        }
        now.store(due, Ordering::Relaxed);
        let (kind, on) = match event {
            Event::Write(on) => {
                let batch = writes(net.rng);
                nodes[on].log_mut().write(&batch)?;
                written += 1;
                (0u8, on)
            }
            Event::Timer(on) => {
                if let Some(msg) = nodes[on].on_timer()? {
                    net.send(on, due, msg);
                }
                // Jitter, so the two timers do not keep one phase for a whole run.
                let next = due + SYNC_MS as i64 + net.rng.below(SYNC_MS / 2) as i64;
                net.queue.at(next, Event::Timer(on));
                (1, on)
            }
            Event::Deliver { to, nth, msg } => {
                net.arrived(to, nth);
                if let Some(reply) = nodes[to].on_msg(msg)? {
                    net.send(to, due, reply);
                }
                (2, to)
            }
        };
        let mut step = Fnv::new();
        step.bytes(&due.to_le_bytes());
        step.bytes(&[kind, on as u8]);
        // A timer changes no state, and a scan per timer is most of a run that cannot settle.
        if kind != 1 {
            hashes[on] = nodes[on].log().state_hash()?;
        }
        step.bytes(&hashes[on].to_le_bytes());
        trace.push(step.finish());
        if written == WRITES && nodes[0].settled()? && nodes[1].settled()? {
            settled = true;
            break;
        }
    }
    let stats = net.stats;
    let [a, b] = &nodes;
    Ok(Outcome {
        settled,
        hashes,
        trace,
        logs: [
            a.log().ops_after(None, i64::MAX)?,
            b.log().ops_after(None, i64::MAX)?,
        ],
        stats,
    })
}
