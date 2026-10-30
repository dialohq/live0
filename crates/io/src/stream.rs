use std::collections::BTreeMap;

use async_trait::async_trait;
use types::{Event, Offset, Partition, Schema, Watermark};

use crate::Error;

pub(crate) const MAX_BATCH: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartitionOffsets {
    pub partition: Partition,
    /// The earliest offset still retained.
    pub low: Offset,
    /// The offset the next record will get.
    pub high: Offset,
}

#[async_trait]
pub trait StreamSource: Send + Sync {
    async fn partitions(&self, topic: &str) -> Result<Vec<PartitionOffsets>, Error>;

    /// Reads every topic and partition in `from`, starting at its offset, decoding by the topic's schema.
    async fn subscribe(
        &self,
        from: &Watermark,
        schemas: &BTreeMap<String, Schema>,
    ) -> Result<Box<dyn EventStream>, Error>;
}

#[async_trait]
pub trait EventStream: Send {
    /// Waits for the next non-empty batch; offsets within a partition are in order and without gaps.
    async fn next(&mut self) -> Result<Vec<Event>, Error>;
}

/// Starting anywhere but `low..=high` would skip events or wait for ones that don't exist yet.
pub(crate) fn check_offset(
    topic: &str,
    partition: Partition,
    offset: Offset,
    known: &[PartitionOffsets],
) -> Result<(), Error> {
    let Some(p) = known.iter().find(|p| p.partition == partition) else {
        return Err(Error::UnknownPartition {
            topic: topic.to_owned(),
            partition,
        });
    };
    match (p.low..=p.high).contains(&offset) {
        true => Ok(()),
        false => Err(Error::OffsetOutOfRange {
            topic: topic.to_owned(),
            partition,
            offset,
            low: p.low,
            high: p.high,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_must_be_retained_or_next() {
        let known = [PartitionOffsets {
            partition: 0,
            low: 3,
            high: 8,
        }];
        assert!(check_offset("t", 0, 3, &known).is_ok());
        assert!(check_offset("t", 0, 8, &known).is_ok());
        assert!(matches!(
            check_offset("t", 0, 2, &known),
            Err(Error::OffsetOutOfRange {
                low: 3,
                high: 8,
                ..
            })
        ));
        assert!(matches!(
            check_offset("t", 0, 9, &known),
            Err(Error::OffsetOutOfRange { .. })
        ));
        assert!(matches!(
            check_offset("t", 1, 0, &known),
            Err(Error::UnknownPartition { partition: 1, .. })
        ));
    }
}
