use std::collections::BTreeMap;

use crate::Event;

pub type Partition = i32;
pub type Offset = i64;

/// Per topic and partition, the next offset that isn't covered yet.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Watermark(BTreeMap<String, BTreeMap<Partition, Offset>>);

impl Watermark {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, topic: &str, partition: Partition) -> Option<Offset> {
        todo!()
    }

    pub fn set(&mut self, topic: &str, partition: Partition, next: Offset) {
        todo!()
    }

    /// Moves the event's partition past it.
    pub fn advance(&mut self, event: &Event) {
        todo!()
    }

    pub fn topic(&self, topic: &str) -> Option<&BTreeMap<Partition, Offset>> {
        todo!()
    }

    pub fn topics(&self) -> impl Iterator<Item = (&str, &BTreeMap<Partition, Offset>)> {
        self.0
            .iter()
            .map(|(topic, offsets)| (topic.as_str(), offsets))
    }

    /// Takes every partition in `other`; for a query reading several topics from several leases.
    pub fn extend(&mut self, other: Watermark) {
        todo!()
    }

    /// Whether every partition is at or past `other`'s.
    pub fn covers(&self, other: &Watermark) -> bool {
        todo!()
    }
}
