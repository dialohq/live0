use std::{collections::BTreeMap, net::SocketAddr, time::Duration};

use serde::Deserialize;

use crate::{
    ClickhouseConfig, ColumnRule, Config, Error, KafkaConfig, ManagerConfig, PauseConfig,
    PostgresConfig, ProviderConfig, RemoteConfig, RowRule, SecurityConfig, ServerConfig,
    SnapshotManagerConfig, SnapshotsConfig, TableConfig, TablesConfig, WorkOsConfig,
};

/// The top-level keys; env variables for anything else aren't config.
pub(crate) const SECTIONS: &[&str] = &[
    "server",
    "clickhouse",
    "kafka",
    "pause",
    "snapshots",
    "snapshot_manager",
    "tables",
    "security",
];

/// The config file's layout, with its defaults; [`Raw::into_config`] turns it into [`Config`].
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Raw {
    clickhouse: RawClickhouse,
    kafka: RawKafka,
    pause: RawPause,
    #[serde(default)]
    snapshots: RawSnapshots,
    #[serde(default)]
    snapshot_manager: RawSnapshotManager,
    #[serde(default)]
    tables: RawTables,
    #[serde(default)]
    security: RawSecurity,
    #[serde(default)]
    server: RawServer,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawClickhouse {
    url: String,
    #[serde(default = "default_user")]
    user: String,
    #[serde(default)]
    password: String,
    #[serde(default = "default_database")]
    database: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawKafka {
    brokers: String,
    #[serde(default = "thirty_seconds", with = "duration")]
    metadata_refresh: Duration,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPause {
    urls: Vec<String>,
    #[serde(default = "thirty_seconds", with = "duration")]
    retry_for: Duration,
}

#[derive(Deserialize, Default, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Mode {
    #[default]
    Embedded,
    Remote,
}

/// Both modes' keys, checked against `mode` once it's known.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawSnapshots {
    #[serde(default)]
    mode: Mode,
    namespace: Option<String>,
    #[serde(default, with = "optional_duration")]
    refresh_interval: Option<Duration>,
    #[serde(default, with = "optional_duration")]
    lease_ttl: Option<Duration>,
    #[serde(default, with = "optional_duration")]
    adopt_grace: Option<Duration>,
    url: Option<String>,
    #[serde(default, with = "optional_duration")]
    renew_interval: Option<Duration>,
}

#[derive(Default)]
struct RawManagerKeys {
    namespace: Option<String>,
    refresh_interval: Option<Duration>,
    lease_ttl: Option<Duration>,
    adopt_grace: Option<Duration>,
}

impl RawManagerKeys {
    fn is_empty(&self) -> bool {
        self.namespace.is_none()
            && self.refresh_interval.is_none()
            && self.lease_ttl.is_none()
            && self.adopt_grace.is_none()
    }

    fn into_config(self) -> ManagerConfig {
        ManagerConfig {
            namespace: self.namespace.unwrap_or_else(|| "live0".to_owned()),
            refresh_interval: self.refresh_interval.unwrap_or(Duration::from_secs(10)),
            lease_ttl: self.lease_ttl.unwrap_or(Duration::from_secs(30)),
            adopt_grace: self.adopt_grace.unwrap_or(Duration::from_secs(60)),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSnapshotManager {
    #[serde(default = "default_manager_listen")]
    listen: SocketAddr,
    namespace: Option<String>,
    #[serde(default, with = "optional_duration")]
    refresh_interval: Option<Duration>,
    #[serde(default, with = "optional_duration")]
    lease_ttl: Option<Duration>,
    #[serde(default, with = "optional_duration")]
    adopt_grace: Option<Duration>,
}

impl Default for RawSnapshotManager {
    fn default() -> Self {
        Self {
            listen: default_manager_listen(),
            namespace: None,
            refresh_interval: None,
            lease_ttl: None,
            adopt_grace: None,
        }
    }
}

macro_rules! manager_keys {
    ($raw:expr) => {
        RawManagerKeys {
            namespace: $raw.namespace,
            refresh_interval: $raw.refresh_interval,
            lease_ttl: $raw.lease_ttl,
            adopt_grace: $raw.adopt_grace,
        }
    };
}

/// Tables are keyed by name, next to `expose_all`.
#[derive(Deserialize, Default)]
struct RawTables {
    #[serde(default)]
    expose_all: bool,
    #[serde(flatten)]
    tables: BTreeMap<String, RawTable>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTable {
    topic: Option<String>,
    grant: Option<String>,
    #[serde(default)]
    columns: BTreeMap<String, String>,
    #[serde(default)]
    rows: BTreeMap<String, String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawSecurity {
    provider: Option<RawProvider>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum RawProvider {
    Postgres(RawPostgres),
    Workos(RawWorkOs),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPostgres {
    url: String,
    publication: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWorkOs {
    #[serde(default = "default_api_key_env")]
    api_key_env: String,
    #[serde(default = "one_minute", with = "duration")]
    poll_interval: Duration,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawServer {
    #[serde(default = "default_server_listen")]
    listen: SocketAddr,
}

impl Default for RawServer {
    fn default() -> Self {
        Self {
            listen: default_server_listen(),
        }
    }
}

fn default_user() -> String {
    "default".to_owned()
}

fn default_database() -> String {
    "default".to_owned()
}

fn default_api_key_env() -> String {
    "WORKOS_API_KEY".to_owned()
}

fn thirty_seconds() -> Duration {
    Duration::from_secs(30)
}

fn one_minute() -> Duration {
    Duration::from_secs(60)
}

fn default_manager_listen() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 9100))
}

fn default_server_listen() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 8080))
}

fn invalid(key: &str, message: &str) -> Error {
    Error::Invalid {
        origin: key.to_owned(),
        message: message.to_owned(),
    }
}

impl Raw {
    pub(crate) fn into_config(self) -> Result<Config, Error> {
        if self.pause.urls.is_empty() {
            return Err(invalid("pause.urls", "needs at least one URL"));
        }
        let embedded = manager_keys!(self.snapshots);
        let snapshots = match self.snapshots.mode {
            Mode::Embedded => {
                if self.snapshots.url.is_some() || self.snapshots.renew_interval.is_some() {
                    return Err(invalid(
                        "snapshots",
                        "`url` and `renew_interval` are only for mode = \"remote\"",
                    ));
                }
                SnapshotsConfig::Embedded(embedded.into_config())
            }
            Mode::Remote => {
                if !embedded.is_empty() {
                    return Err(invalid(
                        "snapshots",
                        "manager keys are only for mode = \"embedded\"",
                    ));
                }
                let url = self
                    .snapshots
                    .url
                    .ok_or_else(|| invalid("snapshots.url", "required with mode = \"remote\""))?;
                SnapshotsConfig::Remote(RemoteConfig {
                    url,
                    renew_interval: self
                        .snapshots
                        .renew_interval
                        .unwrap_or(Duration::from_secs(10)),
                })
            }
        };
        let manager = manager_keys!(self.snapshot_manager).into_config();
        for (key, namespace) in [
            ("snapshot_manager.namespace", &manager.namespace),
            (
                "snapshots.namespace",
                match &snapshots {
                    SnapshotsConfig::Embedded(m) => &m.namespace,
                    SnapshotsConfig::Remote(_) => &manager.namespace,
                },
            ),
        ] {
            if namespace.is_empty() {
                return Err(invalid(key, "can't be empty"));
            }
        }

        Ok(Config {
            clickhouse: ClickhouseConfig {
                url: self.clickhouse.url,
                user: self.clickhouse.user,
                password: self.clickhouse.password,
                database: self.clickhouse.database,
            },
            kafka: KafkaConfig {
                brokers: self.kafka.brokers,
                metadata_refresh: self.kafka.metadata_refresh,
            },
            pause: PauseConfig {
                urls: self.pause.urls,
                retry_for: self.pause.retry_for,
            },
            snapshots,
            snapshot_manager: SnapshotManagerConfig {
                listen: self.snapshot_manager.listen,
                manager,
            },
            tables: TablesConfig {
                expose_all: self.tables.expose_all,
                tables: self
                    .tables
                    .tables
                    .into_iter()
                    .map(|(name, table)| TableConfig {
                        name,
                        topic: table.topic,
                        grant: table.grant,
                        columns: table
                            .columns
                            .into_iter()
                            .map(|(column, grant)| ColumnRule { column, grant })
                            .collect(),
                        rows: table
                            .rows
                            .into_iter()
                            .map(|(column, grant)| RowRule { column, grant })
                            .collect(),
                    })
                    .collect(),
            },
            security: SecurityConfig {
                provider: self.security.provider.map(|provider| match provider {
                    RawProvider::Postgres(p) => ProviderConfig::Postgres(PostgresConfig {
                        url: p.url,
                        publication: p.publication,
                    }),
                    RawProvider::Workos(w) => ProviderConfig::WorkOs(WorkOsConfig {
                        api_key_env: w.api_key_env,
                        poll_interval: w.poll_interval,
                    }),
                }),
            },
            server: ServerConfig {
                listen: self.server.listen,
            },
        })
    }
}

/// `"250ms"`, `"10s"`, `"5m"` or `"1h"`; never zero.
pub(crate) fn parse_duration(raw: &str) -> Result<Duration, String> {
    let split = raw
        .find(|c: char| !c.is_ascii_digit())
        .ok_or_else(|| format!("`{raw}` needs a unit: ms, s, m or h"))?;
    let (number, unit) = raw.split_at(split);
    let number: u64 = number
        .parse()
        .map_err(|_| format!("`{raw}` isn't a duration like \"10s\""))?;
    let duration = match unit {
        "ms" => Duration::from_millis(number),
        "s" => Duration::from_secs(number),
        "m" => Duration::from_secs(number * 60),
        "h" => Duration::from_secs(number * 3600),
        _ => return Err(format!("`{raw}` needs a unit: ms, s, m or h")),
    };
    match duration.is_zero() {
        true => Err(format!("`{raw}` must be more than zero")),
        false => Ok(duration),
    }
}

mod duration {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, de::Error};

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        super::parse_duration(&String::deserialize(d)?).map_err(D::Error::custom)
    }
}

mod optional_duration {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, de::Error};

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<Duration>, D::Error> {
        super::parse_duration(&String::deserialize(d)?)
            .map(Some)
            .map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("250ms"), Ok(Duration::from_millis(250)));
        assert_eq!(parse_duration("10s"), Ok(Duration::from_secs(10)));
        assert_eq!(parse_duration("5m"), Ok(Duration::from_secs(300)));
        assert_eq!(parse_duration("1h"), Ok(Duration::from_secs(3600)));
        for bad in ["10", "1d", "", "s", "-1s", "1.5s", "0s"] {
            assert!(parse_duration(bad).is_err(), "{bad}");
        }
    }
}
