#![allow(dead_code)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use config::{ClickhouseConfig, KafkaConfig};
use io::{ClickhouseClient, EventStream, Statement};
use rdkafka::{
    ClientConfig, Offset as KafkaOffset, TopicPartitionList,
    admin::{AdminClient, AdminOptions, NewTopic, TopicReplication},
    client::DefaultClientContext,
    producer::{FutureProducer, FutureRecord},
};
use types::{Event, Offset, Partition};

const KAFKA_TIMEOUT: Duration = Duration::from_secs(10);

pub fn statement(sql: &str) -> Statement {
    Statement {
        sql: sql.to_owned(),
        params: Vec::new(),
    }
}

/// A name no other test run uses.
pub fn unique(prefix: &str) -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

pub fn clickhouse_config(database: &str) -> ClickhouseConfig {
    ClickhouseConfig {
        url: std::env::var("CLICKHOUSE_URL").expect("CLICKHOUSE_URL, set by `dnvr io-tests`"),
        user: "default".to_owned(),
        password: String::new(),
        database: database.to_owned(),
    }
}

/// A fresh database; only `drop` removes it, so a failed test leaves its tables to look at.
pub struct Database {
    pub name: String,
    pub client: ClickhouseClient,
}

impl Database {
    pub async fn new() -> Self {
        let name = unique("io_test");
        ClickhouseClient::new(&clickhouse_config("default"))
            .execute(&statement(&format!("CREATE DATABASE `{name}`")))
            .await
            .unwrap();
        let client = ClickhouseClient::new(&clickhouse_config(&name));
        Self { name, client }
    }

    pub async fn run(&self, sql: &str) {
        self.client.execute(&statement(sql)).await.unwrap();
    }

    pub async fn drop(self) {
        self.run(&format!("DROP DATABASE `{}`", self.name)).await;
    }
}

pub fn kafka_config() -> KafkaConfig {
    KafkaConfig {
        brokers: std::env::var("KAFKA_BROKERS").expect("KAFKA_BROKERS, set by `dnvr io-tests`"),
        metadata_refresh: Duration::from_secs(1),
    }
}

/// A fresh topic; only `drop` deletes it.
pub struct Topic {
    pub name: String,
    admin: AdminClient<DefaultClientContext>,
    producer: FutureProducer,
}

impl Topic {
    pub async fn new(partitions: i32) -> Self {
        let name = unique("io_test");
        let mut client = ClientConfig::new();
        client.set("bootstrap.servers", kafka_config().brokers);
        let admin: AdminClient<DefaultClientContext> = client.create().unwrap();
        let options = AdminOptions::new().operation_timeout(Some(KAFKA_TIMEOUT));
        let results = admin
            .create_topics(
                &[NewTopic::new(&name, partitions, TopicReplication::Fixed(1))],
                &options,
            )
            .await
            .unwrap();
        for result in results {
            result.unwrap();
        }
        let producer = client.create().unwrap();
        Self {
            name,
            admin,
            producer,
        }
    }

    /// Waits until the record is written; `None` produces a tombstone.
    pub async fn produce(&self, partition: Partition, payload: Option<&str>) -> Offset {
        let mut record = FutureRecord::<(), str>::to(&self.name).partition(partition);
        if let Some(payload) = payload {
            record = record.payload(payload);
        }
        let delivery = self
            .producer
            .send(record, KAFKA_TIMEOUT)
            .await
            .map_err(|(err, _)| err)
            .unwrap();
        delivery.offset
    }

    /// Deletes the partition's records before `before`, as retention would.
    pub async fn trim(&self, partition: Partition, before: Offset) {
        let mut offsets = TopicPartitionList::new();
        offsets
            .add_partition_offset(&self.name, partition, KafkaOffset::Offset(before))
            .unwrap();
        self.admin
            .delete_records(&offsets, &AdminOptions::new())
            .await
            .unwrap();
    }

    pub async fn drop(self) {
        let results = self
            .admin
            .delete_topics(&[&self.name], &AdminOptions::new())
            .await
            .unwrap();
        for result in results {
            result.unwrap();
        }
    }
}

/// The next `n` events, checking that no batch is empty.
pub async fn collect(stream: &mut dyn EventStream, n: usize) -> Vec<Event> {
    let mut events = Vec::new();
    while events.len() < n {
        let batch = tokio::time::timeout(KAFKA_TIMEOUT, stream.next())
            .await
            .unwrap_or_else(|_| panic!("only {} of {n} events arrived", events.len()))
            .unwrap();
        assert!(!batch.is_empty(), "empty batch");
        events.extend(batch);
    }
    assert_eq!(events.len(), n, "more events than expected");
    events
}

pub async fn assert_quiet(stream: &mut dyn EventStream) {
    if let Ok(batch) = tokio::time::timeout(Duration::from_millis(300), stream.next()).await {
        panic!("expected no events, got {batch:?}");
    }
}
