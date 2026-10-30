use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use async_trait::async_trait;
use serde_json::Value as Json;
use tokio::sync::Notify;
use types::{Column, Event, Offset, Partition, Row, Schema, Watermark};

use crate::{
    Error, EventStream, HistorySource, PartitionOffsets, Statement, StreamSource, TableDescription,
    stream::{MAX_BATCH, check_offset},
};

#[derive(Debug, thiserror::Error)]
pub enum FakeError {
    #[error("no table `{0}`")]
    UnknownTable(String),
    #[error("no topic `{0}`")]
    UnknownTopic(String),
    #[error("topic `{topic}` has no partition {partition}")]
    UnknownPartition { topic: String, partition: Partition },
    #[error("topic `{0}` already exists")]
    TopicExists(String),
    #[error("table `{0}` already exists")]
    TableExists(String),
    #[error("table `{0}` doesn't ingest a topic")]
    NotIngesting(String),
    #[error("statement has no `{{name:Identifier}}` table parameter: {0}")]
    NoTableParameter(String),
    #[error("table `{table}` has no column `{column}`")]
    UnknownColumn { table: String, column: String },
}

/// An in-memory Kafka, sink and ClickHouse, shared by the fake ports here and in `snapshot`; its sink decodes every column as an ordinary one.
#[derive(Clone, Default)]
pub struct FakeCluster {
    inner: Arc<Mutex<FakeState>>,
    produced: Arc<Notify>,
}

#[derive(Default)]
struct FakeState {
    topics: BTreeMap<String, Vec<Vec<Json>>>,
    tables: BTreeMap<String, FakeTable>,
}

struct FakeTable {
    description: TableDescription,
    rows: Vec<Row>,
    /// `None` for clones, which never ingest.
    source: Option<Ingest>,
}

struct Ingest {
    topic: String,
    /// The next offset to ingest; a partition that isn't here is at 0.
    next: BTreeMap<Partition, Offset>,
    paused: bool,
}

impl FakeTable {
    fn schema(&self) -> Schema {
        Schema {
            columns: self
                .description
                .columns
                .iter()
                .map(|c| Column::new(c.name.clone(), c.ty.clone()))
                .collect(),
        }
    }
}

impl FakeState {
    fn table(&mut self, table: &str) -> Result<&mut FakeTable, FakeError> {
        self.tables
            .get_mut(table)
            .ok_or_else(|| FakeError::UnknownTable(table.to_owned()))
    }

    fn source(&mut self, table: &str) -> Result<&mut Ingest, FakeError> {
        self.table(table)?
            .source
            .as_mut()
            .ok_or_else(|| FakeError::NotIngesting(table.to_owned()))
    }
}

impl FakeCluster {
    pub fn new() -> Self {
        Self::default()
    }

    fn state(&self) -> MutexGuard<'_, FakeState> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn create_topic(&self, topic: &str, partitions: Partition) -> Result<(), FakeError> {
        let mut state = self.state();
        if state.topics.contains_key(topic) {
            return Err(FakeError::TopicExists(topic.to_owned()));
        }
        let partitions = (0..partitions).map(|_| Vec::new()).collect();
        state.topics.insert(topic.to_owned(), partitions);
        Ok(())
    }

    /// A table ingesting `topic`, as the sink does; the topic may be created later.
    pub fn create_table(
        &self,
        table: &str,
        topic: &str,
        description: TableDescription,
    ) -> Result<(), FakeError> {
        let mut state = self.state();
        if state.tables.contains_key(table) {
            return Err(FakeError::TableExists(table.to_owned()));
        }
        let source = Ingest {
            topic: topic.to_owned(),
            next: BTreeMap::new(),
            paused: false,
        };
        state.tables.insert(
            table.to_owned(),
            FakeTable {
                description,
                rows: Vec::new(),
                source: Some(source),
            },
        );
        Ok(())
    }

    pub fn produce(
        &self,
        topic: &str,
        partition: Partition,
        value: Json,
    ) -> Result<Offset, FakeError> {
        let offset = {
            let mut state = self.state();
            let partitions = state
                .topics
                .get_mut(topic)
                .ok_or_else(|| FakeError::UnknownTopic(topic.to_owned()))?;
            let records = usize::try_from(partition)
                .ok()
                .and_then(|p| partitions.get_mut(p))
                .ok_or_else(|| FakeError::UnknownPartition {
                    topic: topic.to_owned(),
                    partition,
                })?;
            records.push(value);
            records.len() as Offset - 1
        };
        self.produced.notify_waiters();
        Ok(offset)
    }

    /// Runs the sink once: every table that isn't paused ingests everything produced since.
    pub fn ingest(&self) {
        let mut guard = self.state();
        let FakeState { topics, tables } = &mut *guard;
        for table in tables.values_mut() {
            let schema = table.schema();
            let Some(source) = table.source.as_mut().filter(|s| !s.paused) else {
                continue;
            };
            let Some(partitions) = topics.get(&source.topic) else {
                continue;
            };
            for (partition, records) in (0..).zip(partitions) {
                let next = source.next.entry(partition).or_insert(0);
                let start = usize::try_from(*next).unwrap_or(records.len());
                // The sink doesn't write records that don't fit the table; their offsets still pass.
                table.rows.extend(
                    records[start.min(records.len())..]
                        .iter()
                        .filter_map(|json| types::decode_event(json, &schema)),
                );
                *next = records.len() as Offset;
            }
        }
    }

    /// Stops the table's ingestion, returning the next offset it would ingest per partition.
    pub fn pause(&self, table: &str) -> Result<Watermark, FakeError> {
        let mut guard = self.state();
        let FakeState { topics, tables } = &mut *guard;
        let source = tables
            .get_mut(table)
            .ok_or_else(|| FakeError::UnknownTable(table.to_owned()))?
            .source
            .as_mut()
            .ok_or_else(|| FakeError::NotIngesting(table.to_owned()))?;
        source.paused = true;
        let partitions = topics.get(&source.topic).map_or(0, Vec::len);
        let mut watermark = Watermark::new();
        for partition in (0..).take(partitions) {
            let next = source.next.get(&partition).copied().unwrap_or(0);
            watermark.set(&source.topic, partition, next);
        }
        Ok(watermark)
    }

    pub fn resume(&self, table: &str) -> Result<(), FakeError> {
        self.state().source(table)?.paused = false;
        Ok(())
    }

    /// A frozen copy of the table's rows, like `CREATE TABLE … AS` with `ATTACH PARTITION ALL FROM`.
    pub fn clone_table(&self, table: &str, clone: &str) -> Result<(), FakeError> {
        let mut state = self.state();
        if state.tables.contains_key(clone) {
            return Err(FakeError::TableExists(clone.to_owned()));
        }
        let source = state.table(table)?;
        let copy = FakeTable {
            description: source.description.clone(),
            rows: source.rows.clone(),
            source: None,
        };
        state.tables.insert(clone.to_owned(), copy);
        Ok(())
    }

    pub fn drop_table(&self, table: &str) -> Result<(), FakeError> {
        self.state()
            .tables
            .remove(table)
            .map(drop)
            .ok_or_else(|| FakeError::UnknownTable(table.to_owned()))
    }

    pub fn tables(&self) -> Vec<String> {
        self.state().tables.keys().cloned().collect()
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

/// Doesn't run SQL: returns every row of the table bound to the statement's `Identifier` parameter, so it only fits whole-table reads.
pub struct FakeHistory {
    cluster: FakeCluster,
}

#[async_trait]
impl HistorySource for FakeHistory {
    async fn describe(&self, table: &str) -> Result<TableDescription, Error> {
        self.cluster
            .state()
            .tables
            .get(table)
            .map(|t| t.description.clone())
            .ok_or_else(|| Error::UnknownTable(table.to_owned()))
    }

    async fn fetch(&self, statement: &Statement, schema: &Schema) -> Result<Vec<Row>, Error> {
        let name = table_parameter(statement)
            .ok_or_else(|| FakeError::NoTableParameter(statement.sql.clone()))?;
        let state = self.cluster.state();
        let table = state
            .tables
            .get(name)
            .ok_or_else(|| Error::UnknownTable(name.to_owned()))?;
        let positions = schema
            .columns
            .iter()
            .map(|column| {
                table
                    .description
                    .columns
                    .iter()
                    .position(|c| c.name == column.name)
                    .ok_or_else(|| FakeError::UnknownColumn {
                        table: name.to_owned(),
                        column: column.name.clone(),
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(table
            .rows
            .iter()
            .map(|row| positions.iter().map(|&i| row[i].clone()).collect())
            .collect())
    }
}

/// The value of the first `{name:Identifier}` parameter in the statement.
fn table_parameter(statement: &Statement) -> Option<&str> {
    let end = statement.sql.find(":Identifier}")?;
    let start = statement.sql[..end].rfind('{')? + 1;
    let name = &statement.sql[start..end];
    statement
        .params
        .iter()
        .find(|(param, _)| param == name)
        .map(|(_, value)| value.as_str())
}

pub struct FakeStream {
    cluster: FakeCluster,
}

#[async_trait]
impl StreamSource for FakeStream {
    async fn partitions(&self, topic: &str) -> Result<Vec<PartitionOffsets>, Error> {
        let state = self.cluster.state();
        let partitions = state
            .topics
            .get(topic)
            .ok_or_else(|| Error::UnknownTopic(topic.to_owned()))?;
        Ok((0..)
            .zip(partitions)
            .map(|(partition, records)| PartitionOffsets {
                partition,
                low: 0,
                high: records.len() as Offset,
            })
            .collect())
    }

    async fn subscribe(
        &self,
        from: &Watermark,
        schemas: &BTreeMap<String, Schema>,
    ) -> Result<Box<dyn EventStream>, Error> {
        for (topic, offsets) in from.topics() {
            if !schemas.contains_key(topic) {
                return Err(Error::NoSchema(topic.to_owned()));
            }
            let known = self.partitions(topic).await?;
            for (&partition, &offset) in offsets {
                check_offset(topic, partition, offset, &known)?;
            }
        }
        Ok(Box::new(FakeEvents {
            cluster: self.cluster.clone(),
            positions: from.clone(),
            schemas: schemas.clone(),
        }))
    }
}

struct FakeEvents {
    cluster: FakeCluster,
    positions: Watermark,
    schemas: BTreeMap<String, Schema>,
}

impl FakeEvents {
    /// Up to `MAX_BATCH` events from the current positions, partitions in order.
    fn read(&mut self) -> Vec<Event> {
        let state = self.cluster.state();
        let mut events = Vec::new();
        let mut read = Vec::new();
        for (topic, offsets) in self.positions.topics() {
            let (Some(partitions), Some(schema)) =
                (state.topics.get(topic), self.schemas.get(topic))
            else {
                continue;
            };
            for (&partition, &next) in offsets {
                let records = usize::try_from(partition)
                    .ok()
                    .and_then(|p| partitions.get(p))
                    .map_or(&[][..], Vec::as_slice);
                let start = usize::try_from(next)
                    .unwrap_or(records.len())
                    .min(records.len());
                let take = (MAX_BATCH - events.len()).min(records.len() - start);
                events.extend(
                    (next..)
                        .zip(&records[start..start + take])
                        .map(|(offset, json)| Event {
                            topic: topic.to_owned(),
                            partition,
                            offset,
                            row: types::decode_event(json, schema),
                        }),
                );
                read.push((topic.to_owned(), partition, next + take as Offset));
            }
        }
        drop(state);
        for (topic, partition, next) in read {
            self.positions.set(&topic, partition, next);
        }
        events
    }
}

#[async_trait]
impl EventStream for FakeEvents {
    async fn next(&mut self) -> Result<Vec<Event>, Error> {
        loop {
            let produced = self.cluster.produced.clone();
            let notified = produced.notified();
            tokio::pin!(notified);
            // Registered before reading, so a record produced in between still wakes us.
            notified.as_mut().enable();
            let events = self.read();
            if !events.is_empty() {
                return Ok(events);
            }
            notified.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use types::{DataType, Value};

    use super::*;
    use crate::{ColumnDefault, DescribedColumn, TableEngine};

    fn description(columns: &[(&str, &str)]) -> TableDescription {
        TableDescription {
            columns: columns
                .iter()
                .map(|(name, ty)| DescribedColumn {
                    name: (*name).to_owned(),
                    ty: DataType::parse(ty),
                    default: ColumnDefault::None,
                })
                .collect(),
            engine: TableEngine::MergeTree,
        }
    }

    /// Topic `t` with `partitions` and table `events` reading `id` and `name` from it.
    fn cluster(partitions: Partition) -> FakeCluster {
        let cluster = FakeCluster::new();
        cluster.create_topic("t", partitions).unwrap();
        cluster
            .create_table(
                "events",
                "t",
                description(&[("id", "UInt64"), ("name", "String")]),
            )
            .unwrap();
        cluster
    }

    fn produce(cluster: &FakeCluster, partition: Partition, id: u64) {
        let name = format!("n{id}");
        cluster
            .produce("t", partition, json!({"id": id, "name": name}))
            .unwrap();
    }

    fn select(table: &str) -> Statement {
        Statement {
            sql: "SELECT * FROM {table:Identifier}".to_owned(),
            params: vec![("table".to_owned(), table.to_owned())],
        }
    }

    async fn ids(cluster: &FakeCluster, table: &str) -> Vec<Value> {
        let schema = Schema {
            columns: vec![Column::new("id", DataType::parse("UInt64"))],
        };
        cluster
            .history()
            .fetch(&select(table), &schema)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row[0].clone())
            .collect()
    }

    fn ints(ids: impl IntoIterator<Item = i128>) -> Vec<Value> {
        ids.into_iter().map(Value::Int).collect()
    }

    fn watermark(entries: &[(Partition, Offset)]) -> Watermark {
        let mut w = Watermark::new();
        for (partition, next) in entries {
            w.set("t", *partition, *next);
        }
        w
    }

    #[tokio::test]
    async fn ingest_only_when_called() {
        let cluster = cluster(1);
        produce(&cluster, 0, 1);
        assert_eq!(ids(&cluster, "events").await, ints([]));
        cluster.ingest();
        produce(&cluster, 0, 2);
        assert_eq!(ids(&cluster, "events").await, ints([1]));
        cluster.ingest();
        assert_eq!(ids(&cluster, "events").await, ints([1, 2]));
    }

    #[tokio::test]
    async fn ingest_skips_undecodable_but_moves_on() {
        let cluster = cluster(1);
        produce(&cluster, 0, 1);
        cluster.produce("t", 0, json!({"id": "x"})).unwrap();
        produce(&cluster, 0, 3);
        cluster.ingest();
        assert_eq!(ids(&cluster, "events").await, ints([1, 3]));
        assert_eq!(cluster.pause("events").unwrap(), watermark(&[(0, 3)]));
    }

    #[tokio::test]
    async fn pause_returns_ingested_watermark_and_stops_ingest() {
        let cluster = cluster(1);
        produce(&cluster, 0, 1);
        cluster.ingest();
        produce(&cluster, 0, 2);
        assert_eq!(cluster.pause("events").unwrap(), watermark(&[(0, 1)]));
        assert_eq!(cluster.pause("events").unwrap(), watermark(&[(0, 1)]));
        cluster.ingest();
        assert_eq!(ids(&cluster, "events").await, ints([1]));
        cluster.resume("events").unwrap();
        cluster.ingest();
        assert_eq!(ids(&cluster, "events").await, ints([1, 2]));
    }

    #[test]
    fn pause_covers_every_partition() {
        let cluster = cluster(3);
        produce(&cluster, 1, 1);
        produce(&cluster, 1, 2);
        cluster.ingest();
        assert_eq!(
            cluster.pause("events").unwrap(),
            watermark(&[(0, 0), (1, 2), (2, 0)])
        );
    }

    #[test]
    fn pause_errors() {
        let cluster = cluster(1);
        cluster.clone_table("events", "copy").unwrap();
        assert!(matches!(
            cluster.pause("nope"),
            Err(FakeError::UnknownTable(t)) if t == "nope"
        ));
        assert!(matches!(
            cluster.pause("copy"),
            Err(FakeError::NotIngesting(t)) if t == "copy"
        ));
        assert!(matches!(
            cluster.resume("copy"),
            Err(FakeError::NotIngesting(_))
        ));
    }

    #[tokio::test]
    async fn clone_is_frozen() {
        let cluster = cluster(1);
        produce(&cluster, 0, 1);
        cluster.ingest();
        cluster.clone_table("events", "copy").unwrap();
        produce(&cluster, 0, 2);
        cluster.ingest();
        assert_eq!(ids(&cluster, "copy").await, ints([1]));
        assert_eq!(ids(&cluster, "events").await, ints([1, 2]));
        assert!(matches!(
            cluster.clone_table("events", "copy"),
            Err(FakeError::TableExists(t)) if t == "copy"
        ));
        assert!(matches!(
            cluster.clone_table("nope", "other"),
            Err(FakeError::UnknownTable(_))
        ));
    }

    #[test]
    fn drop_and_tables() {
        let cluster = cluster(1);
        cluster.clone_table("events", "b_copy").unwrap();
        cluster.clone_table("events", "a_copy").unwrap();
        assert_eq!(cluster.tables(), ["a_copy", "b_copy", "events"]);
        cluster.drop_table("b_copy").unwrap();
        assert_eq!(cluster.tables(), ["a_copy", "events"]);
        assert!(matches!(
            cluster.drop_table("b_copy"),
            Err(FakeError::UnknownTable(_))
        ));
    }

    #[tokio::test]
    async fn describe_and_fetch() {
        let cluster = cluster(1);
        produce(&cluster, 0, 1);
        cluster.ingest();
        let history = cluster.history();
        assert_eq!(
            history.describe("events").await.unwrap(),
            description(&[("id", "UInt64"), ("name", "String")])
        );
        assert!(matches!(
            history.describe("nope").await,
            Err(Error::UnknownTable(t)) if t == "nope"
        ));

        let schema = Schema {
            columns: vec![
                Column::new("name", DataType::parse("String")),
                Column::new("id", DataType::parse("UInt64")),
            ],
        };
        assert_eq!(
            history.fetch(&select("events"), &schema).await.unwrap(),
            vec![vec![Value::String("n1".to_owned()), Value::Int(1)]]
        );
        assert!(matches!(
            history.fetch(&select("nope"), &schema).await,
            Err(Error::UnknownTable(_))
        ));
        let missing = Schema {
            columns: vec![Column::new("other", DataType::parse("UInt64"))],
        };
        assert!(matches!(
            history.fetch(&select("events"), &missing).await,
            Err(Error::Fake(FakeError::UnknownColumn { column, .. })) if column == "other"
        ));
        let unbound = Statement {
            sql: "SELECT * FROM events".to_owned(),
            params: Vec::new(),
        };
        assert!(matches!(
            history.fetch(&unbound, &schema).await,
            Err(Error::Fake(FakeError::NoTableParameter(_)))
        ));
    }

    #[test]
    fn setup_errors() {
        let cluster = cluster(2);
        assert!(matches!(
            cluster.create_topic("t", 1),
            Err(FakeError::TopicExists(_))
        ));
        assert!(matches!(
            cluster.create_table("events", "t", description(&[])),
            Err(FakeError::TableExists(_))
        ));
        assert!(matches!(
            cluster.produce("nope", 0, json!({})),
            Err(FakeError::UnknownTopic(_))
        ));
        assert!(matches!(
            cluster.produce("t", 2, json!({})),
            Err(FakeError::UnknownPartition { partition: 2, .. })
        ));
        assert_eq!(cluster.produce("t", 1, json!({})).unwrap(), 0);
    }
}
