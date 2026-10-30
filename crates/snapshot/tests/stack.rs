#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use config::{ClickhouseConfig, ManagerConfig, PauseConfig};
use io::{ClickhouseClient, Statement, quote_identifier};
use rdkafka::{
    ClientConfig,
    producer::{FutureProducer, FutureRecord},
};
use snapshot::{
    ClickhouseCloner, CloneInfo, Cloner, Error, HttpPauseServer, Manager, PauseServer, Snapshots,
};
use types::Watermark;

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name}, set by `dnvr stack-tests`"))
}

fn unique(prefix: &str) -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!(
        "{prefix}_{}_{nanos}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

fn clickhouse(database: &str) -> ClickhouseClient {
    ClickhouseClient::new(&ClickhouseConfig {
        url: env("CLICKHOUSE_URL"),
        user: "default".to_owned(),
        password: String::new(),
        database: database.to_owned(),
    })
}

async fn run(client: &ClickhouseClient, sql: &str) {
    client
        .execute(&Statement {
            sql: sql.to_owned(),
            params: Vec::new(),
        })
        .await
        .unwrap();
}

async fn count(client: &ClickhouseClient, table: &str) -> u64 {
    let body = client
        .query(
            &Statement {
                sql: format!("SELECT count() FROM {}", quote_identifier(table)),
                params: Vec::new(),
            },
            "TabSeparated",
        )
        .await
        .unwrap();
    body.trim().parse().unwrap()
}

fn pause_server(urls: Vec<String>) -> HttpPauseServer {
    HttpPauseServer::new(&PauseConfig {
        urls,
        retry_for: Duration::from_secs(30),
    })
}

/// A fresh database; only `drop` removes it, so a failed test leaves its tables to look at.
struct Database {
    name: String,
    client: ClickhouseClient,
}

impl Database {
    async fn new() -> Self {
        let name = unique("snapshot_test");
        run(&clickhouse("default"), &format!("CREATE DATABASE `{name}`")).await;
        let client = clickhouse(&name);
        Self { name, client }
    }

    async fn drop(self) {
        run(&self.client, &format!("DROP DATABASE `{}`", self.name)).await;
    }
}

fn info(name: &str, table: &str) -> CloneInfo {
    let mut watermark = Watermark::new();
    watermark.set("t", 0, 3);
    watermark.set("t", 1, 0);
    CloneInfo {
        name: name.to_owned(),
        table: table.to_owned(),
        watermark,
        taken_at: UNIX_EPOCH + Duration::from_millis(1_800_000_000_000),
    }
}

async fn round_trip(table: &str) {
    let db = Database::new().await;
    let source = quote_identifier(table);
    run(
        &db.client,
        &format!("CREATE TABLE {source} (id UInt64) ENGINE = MergeTree ORDER BY id"),
    )
    .await;
    run(
        &db.client,
        &format!("INSERT INTO {source} VALUES (1), (2), (3)"),
    )
    .await;
    let cloner = ClickhouseCloner::new(db.client.clone());
    let clone = info(&format!("ns__{table}__1_1"), table);
    cloner.create(&clone).await.unwrap();
    run(&db.client, &format!("INSERT INTO {source} VALUES (4)")).await;
    assert_eq!(count(&db.client, &clone.name).await, 3);
    assert_eq!(count(&db.client, table).await, 4);
    assert_eq!(cloner.list("ns").await.unwrap(), [clone.clone()]);
    assert_eq!(cloner.list("other").await.unwrap(), []);
    Cloner::drop(&cloner, &clone.name).await.unwrap();
    assert_eq!(cloner.list("ns").await.unwrap(), []);
    db.drop().await;
}

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr stack-tests`"]
async fn cloner_round_trip() {
    round_trip("src").await;
}

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr stack-tests`"]
async fn cloner_quotes_names() {
    round_trip("odd name-1").await;
}

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr stack-tests`"]
async fn pause_and_resume_events() {
    let server = pause_server(vec![env("PAUSE_URL")]);
    let pause = server.pause("events").await.unwrap();
    let partitions: Vec<_> = pause
        .watermark
        .topic("events")
        .unwrap()
        .keys()
        .copied()
        .collect();
    assert_eq!(partitions, [0, 1, 2]);
    server.resume(&pause).await.unwrap();
    assert!(matches!(server.resume(&pause).await, Err(Error::Pause(_))));
}

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr stack-tests`"]
async fn pause_unknown_table() {
    let server = pause_server(vec![env("PAUSE_URL")]);
    assert!(matches!(
        server.pause("nope").await,
        Err(Error::UnknownTable(t)) if t == "nope"
    ));
}

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr stack-tests`"]
async fn pause_finds_the_leader() {
    let server = pause_server(vec!["http://127.0.0.1:1".to_owned(), env("PAUSE_URL")]);
    let pause = server.pause("events").await.unwrap();
    server.resume(&pause).await.unwrap();
}

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr stack-tests`"]
async fn manager_against_the_stack() {
    let live0 = clickhouse("live0");
    let before = count(&live0, "events").await;
    let producer: FutureProducer = ClientConfig::new()
        .set("bootstrap.servers", env("KAFKA_BROKERS"))
        .create()
        .unwrap();
    let produced = 6;
    for n in 0..produced {
        let payload = format!(
            r#"{{"id": {n}, "name": "stack", "amount": 1.5, "active": true, "payload": {{}}}}"#
        );
        producer
            .send(
                FutureRecord::<(), str>::to("events")
                    .partition(n % 3)
                    .payload(&payload),
                Duration::from_secs(10),
            )
            .await
            .map_err(|(err, _)| err)
            .unwrap();
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while count(&live0, "events").await < before + produced as u64 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the sink didn't ingest in time"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    let namespace = unique("stack");
    let config = ManagerConfig {
        namespace: namespace.clone(),
        refresh_interval: Duration::from_secs(3600),
        lease_ttl: Duration::from_secs(30),
        adopt_grace: Duration::from_secs(60),
    };
    let cloner = Arc::new(ClickhouseCloner::new(live0.clone()));
    let manager = Manager::start(
        &config,
        Arc::new(pause_server(vec![env("PAUSE_URL")])),
        cloner.clone(),
    )
    .await
    .unwrap();
    let lease = Snapshots::embedded(manager)
        .acquire("events", None)
        .await
        .unwrap();
    let below: i64 = lease.watermark().topic("events").unwrap().values().sum();
    assert_eq!(count(&live0, lease.clone_name()).await, below as u64);
    assert!(below as u64 >= before + produced as u64);
    let name = lease.clone_name().to_owned();
    drop(lease);
    Cloner::drop(cloner.as_ref(), &name).await.unwrap();
}
