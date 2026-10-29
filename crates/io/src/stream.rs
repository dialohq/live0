use std::collections::BTreeMap;

use async_trait::async_trait;
use types::{Event, Offset, Partition, Schema, Watermark};

use crate::Error;

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
