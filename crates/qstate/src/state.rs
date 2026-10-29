use std::{sync::Arc, time::SystemTime};

use compiler::Plan;
use types::{Diff, Event, Watermark};

use crate::{Answer, Checkpoint, Error, Fetch, shapes::ShapeState};

/// Waiting for the one fetch it needs before it can go live.
pub struct Starting {
    plan: Arc<Plan>,
    fetch: Fetch,
    /// What the subscriber holds: nothing (fresh), or a result as of this watermark.
    from: Option<Watermark>,
}

/// A started query, fed events, fetch answers and ticks, in any order.
pub struct Live {
    plan: Arc<Plan>,
    watermark: Watermark,
    shape: ShapeState,
}

pub enum QState {
    Live(Live, Needs),
    Starting(Starting),
}

/// What the state needs from its driver next.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Needs {
    pub fetches: Vec<Fetch>,
    /// When it wants its next tick; replaces the previous one, `None` cancels it.
    pub wake_at: Option<SystemTime>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Step {
    pub diffs: Vec<Diff>,
    pub needs: Needs,
}

/// Starts fresh, or from a checkpoint. An unusable checkpoint is an error, never a silent fresh start: retry with `None`.
pub fn start(plan: Arc<Plan>, checkpoint: Option<Checkpoint>) -> Result<QState, Error> {
    todo!()
}

impl Starting {
    pub fn fetch(&self) -> &Fetch {
        &self.fetch
    }

    /// Diffs take the subscriber's result (empty when fresh) to the answer's watermark.
    pub fn start(self, answer: Answer) -> Result<(Live, Step), Error> {
        todo!()
    }
}

impl Live {
    pub fn plan(&self) -> &Arc<Plan> {
        &self.plan
    }

    /// Advances past every event fed, whether its row matched or not.
    pub fn watermark(&self) -> &Watermark {
        &self.watermark
    }

    pub fn feed(&mut self, events: &[Event]) -> Result<Step, Error> {
        todo!()
    }

    pub fn answer(&mut self, answer: Answer) -> Result<Step, Error> {
        todo!()
    }

    pub fn tick(&mut self, now: SystemTime) -> Result<Step, Error> {
        todo!()
    }

    /// `None` for shapes that can't be checkpointed.
    pub fn checkpoint(&self) -> Option<Checkpoint> {
        todo!()
    }
}
