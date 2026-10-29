use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use serde_json::Value as Json;
use types::{Offset, Partition, Row, Schema, Watermark};

use crate::{
    Error, EventStream, HistorySource, PartitionOffsets, Statement, StreamSource, TableDescription,
};

#[derive(Debug, thiserror::Error)]
pub enum FakeError {
    #[error("no table `{0}`")]
    UnknownTable(String),
    #[error("no topic `{0}`")]
    UnknownTopic(String),
}

/// An in-memory Kafka, sink and ClickHouse, shared by the fake ports here and in `snapshot`.
#[derive(Clone, Default)]
pub struct FakeCluster {
    inner: Arc<Mutex<FakeState>>,
}

#[derive(Default)]
struct FakeState {}

impl FakeCluster {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_topic(&self, topic: &str, partitions: Partition) {
        todo!()
    }

    /// A table ingesting `topic`, as the sink does.
    pub fn create_table(&self, table: &str, topic: &str, description: TableDescription) {
        todo!()
    }

    pub fn produce(&self, topic: &str, partition: Partition, value: Json) -> Offset {
        todo!()
    }

    /// Runs the sink once: every table that isn't paused ingests everything produced since.
    pub fn ingest(&self) {
        todo!()
    }

    /// Stops the table's ingestion, returning the next offset it would ingest per partition.
    pub fn pause(&self, table: &str) -> Result<Watermark, FakeError> {
        todo!()
    }

    pub fn resume(&self, table: &str) -> Result<(), FakeError> {
        todo!()
    }

    pub fn clone_table(&self, table: &str, clone: &str) -> Result<(), FakeError> {
        todo!()
    }

    pub fn drop_table(&self, table: &str) -> Result<(), FakeError> {
        todo!()
    }

    pub fn tables(&self) -> Vec<String> {
        todo!()
    }

    pub fn history(&self) -> FakeHistory {
        FakeHistory {
            cluster: self.clone(),
        }
    }

    pub fn stream(&self) -> FakeStream {
        FakeStream {
            cluster: self.clone(),
        }
    }
}

/// Doesn't run SQL: returns every row of the table bound to the statement's `Identifier` parameter.
pub struct FakeHistory {
    cluster: FakeCluster,
}

#[async_trait]
impl HistorySource for FakeHistory {
    async fn describe(&self, table: &str) -> Result<TableDescription, Error> {
        todo!()
    }

    async fn fetch(&self, statement: &Statement, schema: &Schema) -> Result<Vec<Row>, Error> {
        todo!()
    }
}

pub struct FakeStream {
    cluster: FakeCluster,
}

#[async_trait]
impl StreamSource for FakeStream {
    async fn partitions(&self, topic: &str) -> Result<Vec<PartitionOffsets>, Error> {
        todo!()
    }

    async fn subscribe(
        &self,
        from: &Watermark,
        schemas: &BTreeMap<String, Schema>,
    ) -> Result<Box<dyn EventStream>, Error> {
        todo!()
    }
}
