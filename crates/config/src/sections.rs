use std::{net::SocketAddr, time::Duration};

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub clickhouse: ClickhouseConfig,
    pub kafka: KafkaConfig,
    pub pause: PauseConfig,
    pub snapshots: SnapshotsConfig,
    pub snapshot_manager: SnapshotManagerConfig,
    pub tables: TablesConfig,
    pub security: SecurityConfig,
    pub server: ServerConfig,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClickhouseConfig {
    /// ClickHouse HTTP URL
    pub url: String,
    pub user: String,
    pub password: String,
    pub database: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KafkaConfig {
    pub brokers: String,
    pub metadata_refresh: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PauseConfig {
    /// Every replica; whichever is the leader answers.
    pub urls: Vec<String>,
    /// How long `503`s and connection failures are retried.
    pub retry_for: Duration,
}

/// How this process gets table clones.
#[derive(Debug, Clone, PartialEq)]
pub enum SnapshotsConfig {
    Embedded(ManagerConfig),
    Remote(RemoteConfig),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ManagerConfig {
    /// Prefixes clone names, so deployments sharing a ClickHouse don't adopt each other's clones.
    pub namespace: String,
    pub refresh_interval: Duration,
    pub lease_ttl: Duration,
    /// After a restart, how long adopted clones wait for their holders to renew.
    pub adopt_grace: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RemoteConfig {
    pub url: String,
    pub renew_interval: Duration,
}

/// The `snapshot-manager` binary's own settings.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotManagerConfig {
    pub listen: SocketAddr,
    pub manager: ManagerConfig,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TablesConfig {
    /// Every ClickHouse table can be queried, loaded on first use.
    pub expose_all: bool,
    pub tables: Vec<TableConfig>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableConfig {
    pub name: String,
    /// Defaults to the table's name.
    pub topic: Option<String>,
    /// A check grant needed to query the table at all.
    pub grant: Option<String>,
    pub columns: Vec<ColumnRule>,
    pub rows: Vec<RowRule>,
}

/// The column is hidden from subjects without the check grant.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnRule {
    pub column: String,
    pub grant: String,
}

/// Only rows whose `column` is one of the subject's values for the value grant are visible.
#[derive(Debug, Clone, PartialEq)]
pub struct RowRule {
    pub column: String,
    pub grant: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SecurityConfig {
    pub provider: Option<ProviderConfig>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProviderConfig {
    Postgres(PostgresConfig),
    WorkOs(WorkOsConfig),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PostgresConfig {
    pub url: String,
    pub publication: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkOsConfig {
    pub api_key_env: String,
    pub poll_interval: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServerConfig {
    pub listen: SocketAddr,
}
