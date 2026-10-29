use compiler::Plan;
use types::{Diff, Event, Snapshot, Watermark};

pub enum Error {}

pub struct Live {}
pub struct AwaitingSnapshot {}
pub enum QState {
    Live(Live),
    AwaitingSnapshot(AwaitingSnapshot),
}

pub struct Fetch {}

impl QState {
    pub fn new(plan: Plan, resume: Option<Watermark>) -> Result<Self, Error> {
        todo!()
    }
}

impl AwaitingSnapshot {
    pub fn feed(rows: Snapshot) -> Result<Live, Error> {
        todo!()
    }
}

impl Live {
    pub fn feed(events: Vec<Event>) -> Vec<Diff> {
        todo!()
    }
}
