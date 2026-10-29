use std::sync::Arc;

use compiler::{Fingerprint, Plan};
use qstate::{Checkpoint, QState};
use types::{Diff, Schema, Snapshot};

use crate::{Cursor, Error, engine::Inner};

/// One running query: each `next` drives its state until there's something to yield.
pub struct Subscription {
    plan: Arc<Plan>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Iteration {
    pub output: Output,
    /// Where the result stands after this iteration.
    pub cursor: Cursor,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Output {
    /// First, unless resumed.
    Snapshot(Snapshot),
    Diffs(Vec<Diff>),
}

impl Subscription {
    pub(crate) fn new(engine: Arc<Inner>, plan: Arc<Plan>, state: QState) -> Self {
        todo!()
    }

    pub fn schema(&self) -> &Schema {
        self.plan.schema()
    }

    pub fn fingerprint(&self) -> Fingerprint {
        self.plan.fingerprint()
    }

    /// Never ends by itself; an error ends the subscription.
    pub async fn next(&mut self) -> Result<Iteration, Error> {
        todo!()
    }

    /// The state as of the last iteration, for shapes that can be checkpointed.
    pub fn checkpoint(&self) -> Option<Checkpoint> {
        todo!()
    }
}
