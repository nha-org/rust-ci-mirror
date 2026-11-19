//! Adapter of section 3: op exchange (section 14), on the core's sync port. `Node` is the protocol with no I/O, so the sync
//! phase's real transport and `sim`'s simulated one run the same code.
//!
//! ponytail: two replicas only. A cursor by stamp misses an op a third replica brings in below it; the server's numbering
//! of section 14 replaces the cursor in the sync phase.
//! ponytail: a push holds the peer's own ops too, so each op travels back once and every run's message counts include that;
//! the same numbering ends it, and until then it exercises the duplicate path.
use dash_core::{Op, OpLog};

// ponytail: compiled always since only tests take this crate; behind a feature once the binary takes it as a normal dependency.
pub mod sim;

const PAGE: i64 = 64;

#[derive(Clone, Debug, PartialEq)]
pub enum Msg {
    /// A page of the sender's log, right after what the other side acked.
    Push(Vec<Op>),
    /// The other side holds the sender's log through this op.
    Ack(i64, String),
}

pub struct Node {
    log: Box<dyn OpLog>,
    acked: Option<(i64, String)>,
}

impl Node {
    pub fn new(log: Box<dyn OpLog>) -> Self {
        Self { log, acked: None }
    }

    pub fn log(&self) -> &dyn OpLog {
        &*self.log
    }

    pub fn log_mut(&mut self) -> &mut dyn OpLog {
        &mut *self.log
    }

    fn unacked(&self, limit: i64) -> Result<Vec<Op>, String> {
        let after = self.acked.as_ref().map(|(s, w)| (*s, w.as_str()));
        self.log.ops_after(after, limit)
    }

    /// The periodic send, a retransmit until its ack comes: a lost push or a lost ack costs a period, never an op.
    pub fn on_timer(&self) -> Result<Option<Msg>, String> {
        let ops = self.unacked(PAGE)?;
        Ok((!ops.is_empty()).then_some(Msg::Push(ops)))
    }

    /// The reply, if any. An ack only moves forward: an older one can arrive after a newer one.
    pub fn on_msg(&mut self, msg: Msg) -> Result<Option<Msg>, String> {
        match msg {
            Msg::Push(ops) => {
                self.log.apply(&ops)?;
                Ok(ops.last().map(|op| Msg::Ack(op.stamp, op.writer.clone())))
            }
            Msg::Ack(stamp, writer) => {
                let through = Some((stamp, writer));
                if through > self.acked {
                    self.acked = through;
                }
                Ok(None)
            }
        }
    }

    /// The other side has acked this whole log.
    pub fn settled(&self) -> Result<bool, String> {
        Ok(self.unacked(1)?.is_empty())
    }
}
