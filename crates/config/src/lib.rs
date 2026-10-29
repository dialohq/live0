use std::{net::SocketAddr, time::Duration};

pub struct Config {
    server: ServerConfig,
    clickhouse: ClickhouseConfig,
    kafka: KafkaConfig,
}

pub struct ServerConfig {
    listen: SocketAddr,
}

pub struct ClickhouseConfig {
    /// Clickhouse HTTP URL
    url: String,
    user: String,
    password: Option<String>,
    password_env: Option<String>,
}

pub struct KafkaConfig {
    pub brokers: String,
    pub metadata_refresh: Duration,
}
