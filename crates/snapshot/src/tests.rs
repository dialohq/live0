use std::{collections::BTreeMap, sync::Arc, time::Duration};

use config::ManagerConfig;
use io::{
    ColumnDefault, DescribedColumn, FakeCluster, HistorySource, Statement, StreamSource,
    TableDescription, TableEngine,
};
use serde_json::json;
use types::{Column, DataType, Schema, Watermark};

use crate::{
    CloneInfo, Cloner, Error, FakeCloner, FakePause, Lease, Manager, PauseServer, Snapshots,
};

fn manager_config() -> ManagerConfig {
    ManagerConfig {
        namespace: "test".to_owned(),
        refresh_interval: Duration::from_secs(10),
        lease_ttl: Duration::from_secs(30),
        adopt_grace: Duration::from_secs(60),
    }
}

fn schema() -> Schema {
    Schema {
        columns: vec![Column::new("id", DataType::parse("UInt64"))],
    }
}

/// Topic and table `events`, with fakes and an embedded manager over them.
struct Setup {
    cluster: FakeCluster,
    pause: FakePause,
    cloner: FakeCloner,
    snapshots: Snapshots,
}

impl Setup {
    async fn new() -> Self {
        let cluster = FakeCluster::new();
        cluster.create_topic("events", 1).unwrap();
        let description = TableDescription {
            columns: vec![DescribedColumn {
                name: "id".to_owned(),
                ty: DataType::parse("UInt64"),
                default: ColumnDefault::None,
            }],
            engine: TableEngine::MergeTree,
        };
        cluster
            .create_table("events", "events", description)
            .unwrap();
        let (pause, cloner) = (
            FakePause::new(cluster.clone()),
            FakeCloner::new(cluster.clone()),
        );
        let snapshots = Snapshots::embedded(start(&pause, &cloner).await);
        Self {
            cluster,
            pause,
            cloner,
            snapshots,
        }
    }

    fn produce(&self, ids: std::ops::Range<u64>) {
        for id in ids {
            self.cluster
                .produce("events", 0, json!({ "id": id }))
                .unwrap();
        }
    }

    async fn rows(&self, table: &str) -> usize {
        let statement = Statement {
            sql: "SELECT * FROM {table:Identifier}".to_owned(),
            params: vec![("table".to_owned(), table.to_owned())],
        };
        self.cluster
            .history()
            .fetch(&statement, &schema())
            .await
            .unwrap()
            .len()
    }

    async fn acquire(&self, newer_than: Option<&Watermark>) -> Lease {
        match self.snapshots.acquire("events", newer_than).await {
            Ok(lease) => lease,
            Err(err) => panic!("acquire failed: {err}"),
        }
    }

    async fn acquire_err(&self) -> Error {
        match self.snapshots.acquire("events", None).await {
            Ok(_) => panic!("acquire succeeded"),
            Err(err) => err,
        }
    }

    async fn status(&self) -> crate::Status {
        self.snapshots.status().await.unwrap()
    }
}

async fn start(pause: &FakePause, cloner: &FakeCloner) -> Manager {
    Manager::start(
        &manager_config(),
        Arc::new(pause.clone()),
        Arc::new(cloner.clone()),
    )
    .await
    .unwrap()
}

fn watermark(next: i64) -> Watermark {
    let mut w = Watermark::new();
    w.set("events", 0, next);
    w
}

async fn wait(seconds: u64) {
    tokio::time::sleep(Duration::from_secs(seconds)).await;
}

#[tokio::test(start_paused = true)]
async fn fake_pause_counts_tokens() {
    let s = Setup::new().await;
    let (first, second) = (
        s.pause.pause("events").await.unwrap(),
        s.pause.pause("events").await.unwrap(),
    );
    s.produce(0..1);
    s.pause.resume(&first).await.unwrap();
    s.cluster.ingest();
    assert_eq!(s.rows("events").await, 0);
    s.pause.resume(&second).await.unwrap();
    s.cluster.ingest();
    assert_eq!(s.rows("events").await, 1);
    assert!(matches!(
        s.pause.resume(&second).await,
        Err(Error::Pause(_))
    ));
    assert!(matches!(
        s.pause.pause("nope").await,
        Err(Error::UnknownTable(t)) if t == "nope"
    ));
}

#[tokio::test(start_paused = true)]
async fn fake_cloner_lists_by_namespace() {
    let s = Setup::new().await;
    let info = |name: &str| CloneInfo {
        name: name.to_owned(),
        table: "events".to_owned(),
        watermark: watermark(0),
        taken_at: std::time::SystemTime::UNIX_EPOCH,
    };
    s.cloner.create(&info("a__events__1")).await.unwrap();
    s.cloner.create(&info("b__events__1")).await.unwrap();
    assert_eq!(s.cloner.list("a").await.unwrap(), [info("a__events__1")]);
    Cloner::drop(&s.cloner, "a__events__1").await.unwrap();
    assert_eq!(s.cloner.list("a").await.unwrap(), []);
    assert_eq!(s.cluster.tables(), ["b__events__1", "events"]);
}

#[tokio::test(start_paused = true)]
async fn clone_holds_exactly_what_is_below_the_watermark() {
    let s = Setup::new().await;
    s.produce(0..5);
    s.cluster.ingest();
    s.produce(5..8);
    let lease = s.acquire(None).await;
    assert_eq!(lease.table(), "events");
    assert_eq!(lease.watermark(), &watermark(5));
    assert_eq!(s.rows(lease.clone_name()).await, 5);

    let schemas = BTreeMap::from([("events".to_owned(), schema())]);
    let mut events = s
        .cluster
        .stream()
        .subscribe(lease.watermark(), &schemas)
        .await
        .unwrap();
    let rest = events.next().await.unwrap();
    assert_eq!(rest.iter().map(|e| e.offset).collect::<Vec<_>>(), [5, 6, 7]);

    s.cluster.ingest();
    assert_eq!(s.rows("events").await, 8);
    assert_eq!(s.rows(lease.clone_name()).await, 5);
}

#[tokio::test(start_paused = true)]
async fn concurrent_acquires_share_one_clone() {
    let s = Setup::new().await;
    let (a, b) = tokio::join!(s.acquire(None), s.acquire(None));
    assert_eq!(a.clone_name(), b.clone_name());
    assert_eq!(s.pause.pauses(), 1);
}

#[tokio::test(start_paused = true)]
async fn acquire_reuses_current_clone() {
    let s = Setup::new().await;
    let first = s.acquire(None).await;
    let second = s.acquire(None).await;
    assert_eq!(first.clone_name(), second.clone_name());
    assert_eq!(s.pause.pauses(), 1);
}

#[tokio::test(start_paused = true)]
async fn newer_than_takes_a_fresh_clone() {
    let s = Setup::new().await;
    s.produce(0..2);
    s.cluster.ingest();
    let first = s.acquire(None).await;
    s.produce(2..4);
    s.cluster.ingest();
    let fresh = s.acquire(Some(&watermark(4))).await;
    assert_ne!(first.clone_name(), fresh.clone_name());
    assert!(fresh.watermark().covers(&watermark(4)));
    assert_eq!(s.rows(fresh.clone_name()).await, 4);
    let same = s.acquire(Some(&watermark(3))).await;
    assert_eq!(same.clone_name(), fresh.clone_name());
}

#[tokio::test(start_paused = true)]
async fn refresh_retires_held_clone_until_released() {
    let s = Setup::new().await;
    let old = s.acquire(None).await;
    let old_name = old.clone_name().to_owned();
    wait(11).await;
    let table = &s.status().await.tables[0];
    let current = table.current.as_ref().unwrap();
    assert_ne!(current.name, old_name);
    assert_eq!(table.retired.len(), 1);
    assert_eq!(
        (table.retired[0].name.as_str(), table.retired[0].leases),
        (old_name.as_str(), 1)
    );
    assert!(s.cluster.tables().contains(&old_name));

    drop(old);
    wait(10).await;
    assert!(!s.cluster.tables().contains(&old_name));
}

#[tokio::test(start_paused = true)]
async fn idle_table_is_dropped_on_refresh() {
    let s = Setup::new().await;
    drop(s.acquire(None).await);
    assert_eq!(s.cluster.tables().len(), 2);
    wait(11).await;
    assert_eq!(s.cluster.tables(), ["events"]);
    assert!(s.status().await.tables.is_empty());
}

#[tokio::test(start_paused = true)]
async fn clone_failure_still_resumes() {
    let s = Setup::new().await;
    s.cloner.set_failing(true);
    assert!(matches!(s.acquire_err().await, Error::Clone(_)));
    s.produce(0..1);
    s.cluster.ingest();
    assert_eq!(s.rows("events").await, 1);
    let status = s.status().await;
    assert!(status.tables[0].last_error.is_some());
    s.cloner.set_failing(false);
    assert_eq!(s.rows(s.acquire(None).await.clone_name()).await, 1);
}

#[tokio::test(start_paused = true)]
async fn pause_failure_is_reported() {
    let s = Setup::new().await;
    s.pause.set_failing(true);
    assert!(matches!(s.acquire_err().await, Error::Pause(_)));
    let status = s.status().await;
    assert!(
        status.tables[0]
            .last_error
            .as_deref()
            .is_some_and(|e| e.contains("failing"))
    );
    assert_eq!(s.cluster.tables(), ["events"]);
}

#[tokio::test(start_paused = true)]
async fn start_adopts_then_drops_after_grace() {
    let s = Setup::new().await;
    let name = s.acquire(None).await.clone_name().to_owned();
    let Setup {
        cluster,
        pause,
        cloner,
        snapshots,
    } = s;
    drop(snapshots);

    let restarted = start(&pause, &cloner).await;
    let status = restarted.status().await;
    let table = &status.tables[0];
    assert!(table.current.is_none());
    assert_eq!(
        table
            .retired
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        [name.as_str()]
    );
    wait(30).await;
    assert!(cluster.tables().contains(&name));
    wait(31).await;
    assert!(!cluster.tables().contains(&name));
}

#[tokio::test(start_paused = true)]
async fn status_counts_leases() {
    let s = Setup::new().await;
    let a = s.acquire(None).await;
    let b = s.acquire(None).await;
    let leases = |status: crate::Status| status.tables[0].current.as_ref().unwrap().leases;
    assert_eq!(leases(s.status().await), 2);
    drop(a);
    assert_eq!(leases(s.status().await), 1);
    drop(b);
    assert_eq!(leases(s.status().await), 0);
}
