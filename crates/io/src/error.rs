use types::{Offset, Partition};

use crate::{ClickhouseError, FakeError, KafkaError};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no table `{0}`")]
    UnknownTable(String),
    #[error("no topic `{0}`")]
    UnknownTopic(String),
    #[error("topic `{topic}` has no partition {partition}")]
    UnknownPartition { topic: String, partition: Partition },
    #[error("no schema for topic `{0}`")]
    NoSchema(String),
    #[error("offset {offset} of `{topic}` partition {partition} is outside {low}..={high}")]
    OffsetOutOfRange {
        topic: String,
        partition: Partition,
        offset: Offset,
        low: Offset,
        high: Offset,
    },
    #[error(transparent)]
    Clickhouse(#[from] ClickhouseError),
    #[error(transparent)]
    Kafka(#[from] KafkaError),
    #[error(transparent)]
    Fake(#[from] FakeError),
}
