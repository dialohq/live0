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
        self.0.get(topic)?.get(&partition).copied()
    }

    pub fn set(&mut self, topic: &str, partition: Partition, next: Offset) {
        self.0
            .entry(topic.to_owned())
            .or_default()
            .insert(partition, next);
    }

    /// Moves the event's partition past it.
    pub fn advance(&mut self, event: &Event) {
        self.set(&event.topic, event.partition, event.offset + 1);
    }

    pub fn topic(&self, topic: &str) -> Option<&BTreeMap<Partition, Offset>> {
        self.0.get(topic)
    }

    pub fn topics(&self) -> impl Iterator<Item = (&str, &BTreeMap<Partition, Offset>)> {
        self.0
            .iter()
            .map(|(topic, offsets)| (topic.as_str(), offsets))
    }

    /// Takes every partition in `other`; for a query reading several topics from several leases.
    pub fn extend(&mut self, other: Watermark) {
        for (topic, offsets) in other.0 {
            self.0.entry(topic).or_default().extend(offsets);
        }
    }

    /// Whether every partition is at or past `other`'s.
    pub fn covers(&self, other: &Watermark) -> bool {
        other.topics().all(|(topic, offsets)| {
            offsets
                .iter()
                .all(|(p, next)| self.get(topic, *p).is_some_and(|mine| mine >= *next))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watermark(entries: &[(&str, Partition, Offset)]) -> Watermark {
        let mut w = Watermark::new();
        for (topic, partition, next) in entries {
            w.set(topic, *partition, *next);
        }
        w
    }

    #[test]
    fn set_get_and_topic() {
        let w = watermark(&[("a", 0, 5), ("a", 1, 7), ("b", 0, 1), ("a", 0, 6)]);
        assert_eq!(w.get("a", 0), Some(6));
        assert_eq!(w.get("a", 2), None);
        assert_eq!(w.get("c", 0), None);
        assert_eq!(w.topic("a"), Some(&BTreeMap::from([(0, 6), (1, 7)])));
        assert_eq!(w.topics().map(|(t, _)| t).collect::<Vec<_>>(), ["a", "b"]);
    }

    #[test]
    fn advance_moves_past_the_event() {
        let mut w = watermark(&[("a", 0, 3)]);
        w.advance(&Event {
            topic: "a".to_owned(),
            partition: 0,
            offset: 9,
            row: None,
        });
        assert_eq!(w.get("a", 0), Some(10));
    }

    #[test]
    fn extend_takes_the_other_partitions() {
        let mut w = watermark(&[("a", 0, 5), ("a", 1, 7)]);
        w.extend(watermark(&[("a", 1, 2), ("b", 0, 1)]));
        assert_eq!(w, watermark(&[("a", 0, 5), ("a", 1, 2), ("b", 0, 1)]));
    }

    #[test]
    fn covers() {
        let w = watermark(&[("a", 0, 5), ("a", 1, 7)]);
        assert!(w.covers(&watermark(&[("a", 0, 5), ("a", 1, 3)])));
        assert!(w.covers(&Watermark::new()));
        assert!(!w.covers(&watermark(&[("a", 0, 6)])));
        assert!(!w.covers(&watermark(&[("a", 2, 0)])));
        assert!(!w.covers(&watermark(&[("b", 0, 0)])));
        assert!(!Watermark::new().covers(&w));
    }
}
