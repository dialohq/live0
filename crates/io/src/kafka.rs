use std::collections::BTreeMap;

use async_trait::async_trait;
use types::{Schema, Watermark};

use crate::{Error, EventStream, PartitionOffsets, StreamSource};

#[derive(Debug, thiserror::Error)]
pub enum KafkaError {}

pub struct KafkaStream {}

impl KafkaStream {
    pub fn new(config: &config::KafkaConfig) -> Result<Self, KafkaError> {
        todo!()
    }
}

#[async_trait]
impl StreamSource for KafkaStream {
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
