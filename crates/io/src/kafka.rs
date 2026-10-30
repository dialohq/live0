use std::{collections::BTreeMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use rdkafka::{
    ClientConfig, Message, Offset as KafkaOffset, TopicPartitionList,
    consumer::{BaseConsumer, Consumer, StreamConsumer},
    error::KafkaError as ClientError,
    types::RDKafkaErrorCode,
};
use types::{Event, Offset, Partition, Schema, Watermark};

use crate::{Error, EventStream, PartitionOffsets, StreamSource};

const METADATA_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_BATCH: usize = 1000;

#[derive(Debug, thiserror::Error)]
pub enum KafkaError {
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error("a blocking Kafka call failed to run: {0}")]
    Blocking(#[from] tokio::task::JoinError),
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
}

/// Reads partitions at the offsets it's given; nothing is committed, so it never joins a group.
pub struct KafkaStream {
    config: ClientConfig,
    metadata: Arc<BaseConsumer>,
}

impl KafkaStream {
    pub fn new(config: &config::KafkaConfig) -> Result<Self, KafkaError> {
        let mut client = ClientConfig::new();
        client
            .set("bootstrap.servers", &config.brokers)
            // librdkafka requires one even when partitions are assigned by hand.
            .set("group.id", format!("live0-{}", std::process::id()))
            .set("enable.auto.commit", "false")
            .set("enable.auto.offset.store", "false")
            .set("allow.auto.create.topics", "false")
            .set("auto.offset.reset", "error")
            .set(
                "topic.metadata.refresh.interval.ms",
                config.metadata_refresh.as_millis().to_string(),
            )
            .set("fetch.wait.max.ms", "10");
        Ok(Self {
            metadata: Arc::new(client.create()?),
            config: client,
        })
    }
}

#[async_trait]
impl StreamSource for KafkaStream {
    async fn partitions(&self, topic: &str) -> Result<Vec<PartitionOffsets>, Error> {
        let (consumer, topic) = (self.metadata.clone(), topic.to_owned());
        // librdkafka's metadata calls block until the broker answers.
        let offsets = tokio::task::spawn_blocking(move || partitions(&consumer, &topic))
            .await
            .map_err(KafkaError::from)??;
        Ok(offsets)
    }

    async fn subscribe(
        &self,
        from: &Watermark,
        schemas: &BTreeMap<String, Schema>,
    ) -> Result<Box<dyn EventStream>, Error> {
        let mut assignment = TopicPartitionList::new();
        for (topic, offsets) in from.topics() {
            if !schemas.contains_key(topic) {
                return Err(KafkaError::NoSchema(topic.to_owned()).into());
            }
            let known = self.partitions(topic).await?;
            for (&partition, &offset) in offsets {
                check_offset(topic, partition, offset, &known)?;
                assignment
                    .add_partition_offset(topic, partition, KafkaOffset::Offset(offset))
                    .map_err(KafkaError::from)?;
            }
        }
        let consumer: StreamConsumer = self.config.create().map_err(KafkaError::from)?;
        consumer.assign(&assignment).map_err(KafkaError::from)?;
        Ok(Box::new(KafkaEvents {
            consumer,
            schemas: schemas.clone(),
        }))
    }
}

fn partitions(consumer: &BaseConsumer, topic: &str) -> Result<Vec<PartitionOffsets>, KafkaError> {
    let metadata = consumer.fetch_metadata(Some(topic), METADATA_TIMEOUT)?;
    let Some(found) = metadata.topics().iter().find(|t| t.name() == topic) else {
        return Err(KafkaError::UnknownTopic(topic.to_owned()));
    };
    match found.error().map(RDKafkaErrorCode::from) {
        Some(RDKafkaErrorCode::UnknownTopicOrPartition) => {
            return Err(KafkaError::UnknownTopic(topic.to_owned()));
        }
        Some(code) => return Err(ClientError::MetadataFetch(code).into()),
        None => {}
    }
    let mut offsets = found
        .partitions()
        .iter()
        .map(|p| {
            let (low, high) = consumer.fetch_watermarks(topic, p.id(), METADATA_TIMEOUT)?;
            Ok(PartitionOffsets {
                partition: p.id(),
                low,
                high,
            })
        })
        .collect::<Result<Vec<_>, KafkaError>>()?;
    offsets.sort_by_key(|p| p.partition);
    Ok(offsets)
}

/// Starting anywhere but `low..=high` would skip events or wait for ones that don't exist yet.
fn check_offset(
    topic: &str,
    partition: Partition,
    offset: Offset,
    known: &[PartitionOffsets],
) -> Result<(), KafkaError> {
    let Some(p) = known.iter().find(|p| p.partition == partition) else {
        return Err(KafkaError::UnknownPartition {
            topic: topic.to_owned(),
            partition,
        });
    };
    match (p.low..=p.high).contains(&offset) {
        true => Ok(()),
        false => Err(KafkaError::OffsetOutOfRange {
            topic: topic.to_owned(),
            partition,
            offset,
            low: p.low,
            high: p.high,
        }),
    }
}

struct KafkaEvents {
    consumer: StreamConsumer,
    schemas: BTreeMap<String, Schema>,
}

impl KafkaEvents {
    fn event(&self, message: &impl Message) -> Result<Event, KafkaError> {
        let schema = self
            .schemas
            .get(message.topic())
            .ok_or_else(|| KafkaError::NoSchema(message.topic().to_owned()))?;
        Ok(decode(
            message.topic(),
            message.partition(),
            message.offset(),
            message.payload(),
            schema,
        ))
    }
}

#[async_trait]
impl EventStream for KafkaEvents {
    async fn next(&mut self) -> Result<Vec<Event>, Error> {
        let first = self.consumer.recv().await.map_err(KafkaError::from)?;
        let mut events = vec![self.event(&first)?];
        drop(first);
        while events.len() < MAX_BATCH {
            let Ok(message) = tokio::time::timeout(Duration::ZERO, self.consumer.recv()).await
            else {
                break;
            };
            events.push(self.event(&message.map_err(KafkaError::from)?)?);
        }
        Ok(events)
    }
}

/// A record that doesn't decode by the schema, tombstones included, keeps its offset with no row.
fn decode(
    topic: &str,
    partition: Partition,
    offset: Offset,
    payload: Option<&[u8]>,
    schema: &Schema,
) -> Event {
    let row = payload
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .and_then(types::parse_event)
        .and_then(|json| types::decode_event(&json, schema));
    Event {
        topic: topic.to_owned(),
        partition,
        offset,
        row,
    }
}

#[cfg(test)]
mod tests {
    use types::{Column, DataType, Value};

    use super::*;

    fn row(payload: Option<&[u8]>) -> Option<Vec<Value>> {
        let schema = Schema {
            columns: vec![
                Column::new("id", DataType::parse("UInt64")),
                Column::new("name", DataType::parse("String")),
            ],
        };
        let event = decode("t", 2, 7, payload, &schema);
        assert_eq!(
            (event.topic.as_str(), event.partition, event.offset),
            ("t", 2, 7)
        );
        event.row
    }

    #[test]
    fn decodes_by_schema() {
        assert_eq!(
            row(Some(br#"{"id": 007, "name": "a", "extra": 1}"#)),
            Some(vec![Value::Int(7), Value::String("a".to_owned())])
        );
    }

    #[test]
    fn undecodable_records_have_no_row() {
        for payload in [
            None,
            Some(&b"\xff\xfe"[..]),
            Some(b"{\"id\": nope}"),
            Some(b"[1, 2]"),
            Some(b"{\"id\": \"x\"}"),
        ] {
            assert_eq!(row(payload), None, "{payload:?}");
        }
    }

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
            Err(KafkaError::OffsetOutOfRange {
                low: 3,
                high: 8,
                ..
            })
        ));
        assert!(matches!(
            check_offset("t", 0, 9, &known),
            Err(KafkaError::OffsetOutOfRange { .. })
        ));
        assert!(matches!(
            check_offset("t", 1, 0, &known),
            Err(KafkaError::UnknownPartition { partition: 1, .. })
        ));
    }
}
