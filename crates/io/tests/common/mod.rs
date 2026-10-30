use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use config::ClickhouseConfig;
use io::{ClickhouseClient, Statement};

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
