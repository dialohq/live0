use crate::{ClickhouseError, FakeError, KafkaError};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Clickhouse(#[from] ClickhouseError),
    #[error(transparent)]
    Kafka(#[from] KafkaError),
    #[error(transparent)]
    Fake(#[from] FakeError),
}
