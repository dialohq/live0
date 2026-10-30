use std::{path::PathBuf, str::FromStr};

use figment::{
    Figment, Metadata, Profile, Provider,
    providers::{Format, Toml},
    value::{Dict, Map, Value},
};

use crate::{
    Config,
    raw::{Raw, SECTIONS},
};

/// Environment variables are read as `LIVE0_<SECTION>__<KEY>`, e.g. `LIVE0_CLICKHOUSE__URL`.
pub const ENV_PREFIX: &str = "LIVE0_";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("can't read config file {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{origin}: {message}")]
    Invalid { origin: String, message: String },
}

/// Where the effective config comes from; earlier fields take precedence, then defaults.
#[derive(Debug, Clone, Default)]
pub struct Sources {
    /// Dotted keys, e.g. `("server.listen", "0.0.0.0:8080")`, from a binary's flags.
    pub explicit: Vec<(String, String)>,
    pub env: Vec<(String, String)>,
    pub file: Option<PathBuf>,
}

impl Sources {
    /// Takes the process environment variables that start with [`ENV_PREFIX`].
    pub fn from_process(file: Option<PathBuf>, explicit: Vec<(String, String)>) -> Self {
        Self {
            explicit,
            env: std::env::vars()
                .filter(|(name, _)| name.starts_with(ENV_PREFIX))
                .collect(),
            file,
        }
    }
}

/// Merges the sources and checks the result on its own, e.g. an empty `pause.urls`.
pub fn load(sources: &Sources) -> Result<Config, Error> {
    let mut figment = Figment::new();
    if let Some(path) = &sources.file {
        let text = std::fs::read_to_string(path).map_err(|source| Error::Read {
            path: path.clone(),
            source,
        })?;
        let origin = format!("file {}", path.display());
        let data = Toml::string(&text)
            .data()
            .map_err(|err| invalid(&origin, &err))?
            .remove(&Profile::Default)
            .unwrap_or_default();
        figment = figment.merge(Layer { origin, data });
    }
    for (name, value) in &sources.env {
        if let Some(key) = env_key(name) {
            figment = figment.merge(Layer::value(format!("env {name}"), &key, value));
        }
    }
    for (key, value) in &sources.explicit {
        figment = figment.merge(Layer::value(format!("flag {key}"), key, value));
    }
    let raw: Raw = figment.extract().map_err(|err| {
        let origin = err
            .metadata
            .as_ref()
            .map_or_else(|| "config".to_owned(), |m| m.name.to_string());
        invalid(&origin, &err)
    })?;
    raw.into_config()
}

/// `LIVE0_TABLES__EVENTS__TOPIC` is `tables.events.topic`; `None` unless it starts with a section.
fn env_key(name: &str) -> Option<String> {
    let key = name
        .strip_prefix(ENV_PREFIX)?
        .to_lowercase()
        .replace("__", ".");
    let section = key.split('.').next()?;
    (SECTIONS.contains(&section) && key.contains('.')).then_some(key)
}

fn invalid(origin: &str, err: &figment::Error) -> Error {
    let message = match err.path.is_empty() {
        true => err.kind.to_string(),
        false => format!("`{}`: {}", err.path.join("."), err.kind),
    };
    Error::Invalid {
        origin: origin.to_owned(),
        message,
    }
}

/// One source's keys, named so errors say where a value came from.
struct Layer {
    origin: String,
    data: Dict,
}

impl Layer {
    /// A dotted key whose value is read as TOML when it can be, otherwise as a string.
    fn value(origin: String, key: &str, raw: &str) -> Self {
        let value = Value::from_str(raw).unwrap_or_else(|never| match never {});
        let data = figment::util::nest(key, value)
            .into_dict()
            .unwrap_or_default();
        Self { origin, data }
    }
}

impl Provider for Layer {
    fn metadata(&self) -> Metadata {
        Metadata::named(self.origin.clone())
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        Ok(Profile::Default.collect(self.data.clone()))
    }
}

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, time::Duration};

    use super::*;
    use crate::{
        ClickhouseConfig, ColumnRule, KafkaConfig, ManagerConfig, PauseConfig, PostgresConfig,
        ProviderConfig, RemoteConfig, RowRule, SecurityConfig, ServerConfig, SnapshotManagerConfig,
        SnapshotsConfig, TableConfig, TablesConfig, WorkOsConfig,
    };

    const REQUIRED: &str = r#"
        [clickhouse]
        url = "http://ch:8123"
        [kafka]
        brokers = "k:9092"
        [pause]
        urls = ["http://pause:1"]
    "#;

    /// A config file with `text`, removed when dropped.
    struct File(PathBuf);

    impl File {
        fn new(text: &str) -> Self {
            static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("live0-config-{}-{n}.toml", std::process::id()));
            std::fs::write(&path, text).unwrap();
            Self(path)
        }

        fn sources(&self) -> Sources {
            Sources {
                file: Some(self.0.clone()),
                ..Sources::default()
            }
        }
    }

    impl Drop for File {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn load_text(text: &str) -> Result<Config, Error> {
        load(&File::new(text).sources())
    }

    fn pairs(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn invalid_message(result: Result<Config, Error>) -> (String, String) {
        match result {
            Err(Error::Invalid { origin, message }) => (origin, message),
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    fn manager_defaults() -> ManagerConfig {
        ManagerConfig {
            namespace: "live0".to_owned(),
            refresh_interval: Duration::from_secs(10),
            lease_ttl: Duration::from_secs(30),
            adopt_grace: Duration::from_secs(60),
        }
    }

    #[test]
    fn example_loads() {
        let path = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/live0.example.toml"));
        let config = load(&Sources {
            file: Some(path),
            ..Sources::default()
        })
        .unwrap();
        assert_eq!(
            config.snapshots,
            SnapshotsConfig::Embedded(manager_defaults())
        );
        assert_eq!(config.snapshot_manager.manager, manager_defaults());
        assert_eq!(
            config.tables.tables,
            vec![
                TableConfig {
                    name: "events".to_owned(),
                    topic: None,
                    grant: None,
                    columns: Vec::new(),
                    rows: Vec::new(),
                },
                TableConfig {
                    name: "orders".to_owned(),
                    topic: Some("prod.orders".to_owned()),
                    grant: Some("orders:read".to_owned()),
                    columns: vec![ColumnRule {
                        column: "email".to_owned(),
                        grant: "pii:read".to_owned(),
                    }],
                    rows: vec![RowRule {
                        column: "tenant_id".to_owned(),
                        grant: "tenant".to_owned(),
                    }],
                },
            ]
        );
        assert_eq!(
            config.security.provider,
            Some(ProviderConfig::Postgres(PostgresConfig {
                url: "postgres://live0@127.0.0.1/live0".to_owned(),
                publication: "live0_grants".to_owned(),
            }))
        );
    }

    #[test]
    fn minimal_gets_defaults() {
        let config = load_text(REQUIRED).unwrap();
        assert_eq!(
            config,
            Config {
                clickhouse: ClickhouseConfig {
                    url: "http://ch:8123".to_owned(),
                    user: "default".to_owned(),
                    password: String::new(),
                    database: "default".to_owned(),
                },
                kafka: KafkaConfig {
                    brokers: "k:9092".to_owned(),
                    metadata_refresh: Duration::from_secs(30),
                },
                pause: PauseConfig {
                    urls: vec!["http://pause:1".to_owned()],
                    retry_for: Duration::from_secs(30),
                },
                snapshots: SnapshotsConfig::Embedded(manager_defaults()),
                snapshot_manager: SnapshotManagerConfig {
                    listen: SocketAddr::from(([127, 0, 0, 1], 9100)),
                    manager: manager_defaults(),
                },
                tables: TablesConfig {
                    expose_all: false,
                    tables: Vec::new(),
                },
                security: SecurityConfig { provider: None },
                server: ServerConfig {
                    listen: SocketAddr::from(([127, 0, 0, 1], 8080)),
                },
            }
        );
    }

    #[test]
    fn precedence() {
        let file = File::new(&format!("{REQUIRED}\n[server]\nlisten = \"0.0.0.0:1\"\n"));
        let listen = |env: &[(&str, &str)], flags: &[(&str, &str)]| {
            load(&Sources {
                env: pairs(env),
                explicit: pairs(flags),
                ..file.sources()
            })
            .unwrap()
            .server
            .listen
            .port()
        };
        assert_eq!(listen(&[], &[]), 1);
        assert_eq!(listen(&[("LIVE0_SERVER__LISTEN", "0.0.0.0:2")], &[]), 2);
        assert_eq!(
            listen(
                &[("LIVE0_SERVER__LISTEN", "0.0.0.0:2")],
                &[("server.listen", "0.0.0.0:3")]
            ),
            3
        );
    }

    #[test]
    fn env_names() {
        let file = File::new(&format!("{REQUIRED}\n[tables.events]\n"));
        let config = load(&Sources {
            env: pairs(&[
                ("LIVE0_KAFKA__BROKERS", "a:1,b:2"),
                ("LIVE0_KAFKA__METADATA_REFRESH", "5s"),
                ("LIVE0_TABLES__EVENTS__TOPIC", "prod.events"),
                ("LIVE0_E2E", "1"),
                ("LIVE0_CLICKHOUSE_URL", "ignored"),
            ]),
            ..file.sources()
        })
        .unwrap();
        assert_eq!(config.kafka.brokers, "a:1,b:2");
        assert_eq!(config.kafka.metadata_refresh, Duration::from_secs(5));
        assert_eq!(
            config.tables.tables[0].topic.as_deref(),
            Some("prod.events")
        );
        assert_eq!(config.clickhouse.url, "http://ch:8123");
    }

    #[test]
    fn flag_values_read_as_toml() {
        let file = File::new(REQUIRED);
        let config = load(&Sources {
            explicit: pairs(&[
                ("pause.urls", r#"["http://a", "http://b"]"#),
                ("tables.expose_all", "true"),
            ]),
            ..file.sources()
        })
        .unwrap();
        assert_eq!(config.pause.urls, ["http://a", "http://b"]);
        assert!(config.tables.expose_all);
    }

    #[test]
    fn unknown_key_is_named() {
        let file = File::new(&REQUIRED.replace("brokers", "broker"));
        let (origin, message) = invalid_message(load(&file.sources()));
        assert!(origin.starts_with("file "), "{origin}");
        assert!(message.contains("`broker`"), "{message}");
        let file = File::new(REQUIRED);
        let (origin, message) = invalid_message(load(&Sources {
            explicit: pairs(&[("server.port", "1")]),
            ..file.sources()
        }));
        assert_eq!(origin, "flag server.port");
        assert!(message.contains("`port`"), "{message}");
    }

    #[test]
    fn missing_required() {
        let (_, message) = invalid_message(load_text(&REQUIRED.replace("url =", "uri =")));
        assert!(
            message.contains("url") || message.contains("uri"),
            "{message}"
        );
        let (_, message) = invalid_message(load_text(
            "[kafka]\nbrokers = \"k\"\n[pause]\nurls = [\"p\"]\n",
        ));
        assert!(message.contains("clickhouse"), "{message}");
    }

    #[test]
    fn remote_snapshots() {
        let config = load_text(&format!(
            "{REQUIRED}\n[snapshots]\nmode = \"remote\"\nurl = \"http://manager:9100\"\n"
        ))
        .unwrap();
        assert_eq!(
            config.snapshots,
            SnapshotsConfig::Remote(RemoteConfig {
                url: "http://manager:9100".to_owned(),
                renew_interval: Duration::from_secs(10),
            })
        );
        let (origin, _) = invalid_message(load_text(&format!(
            "{REQUIRED}\n[snapshots]\nmode = \"remote\"\nurl = \"u\"\nnamespace = \"x\"\n"
        )));
        assert_eq!(origin, "snapshots");
        let (origin, _) = invalid_message(load_text(&format!(
            "{REQUIRED}\n[snapshots]\nmode = \"remote\"\n"
        )));
        assert_eq!(origin, "snapshots.url");
        let (origin, _) = invalid_message(load_text(&format!(
            "{REQUIRED}\n[snapshots]\nrenew_interval = \"5s\"\n"
        )));
        assert_eq!(origin, "snapshots");
    }

    #[test]
    fn workos_provider() {
        let config = load_text(&format!(
            "{REQUIRED}\n[security.provider]\nkind = \"workos\"\n"
        ))
        .unwrap();
        assert_eq!(
            config.security.provider,
            Some(ProviderConfig::WorkOs(WorkOsConfig {
                api_key_env: "WORKOS_API_KEY".to_owned(),
                poll_interval: Duration::from_secs(60),
            }))
        );
    }

    #[test]
    fn checks() {
        let (origin, _) =
            invalid_message(load_text(&REQUIRED.replace(r#"["http://pause:1"]"#, "[]")));
        assert_eq!(origin, "pause.urls");
        let (origin, _) = invalid_message(load_text(&format!(
            "{REQUIRED}\n[snapshot_manager]\nnamespace = \"\"\n"
        )));
        assert_eq!(origin, "snapshot_manager.namespace");
        let (_, message) = invalid_message(load_text(&format!(
            "{REQUIRED}\n[snapshots]\nlease_ttl = \"0s\"\n"
        )));
        assert!(message.contains("more than zero"), "{message}");
    }

    #[test]
    fn unreadable_file() {
        let err = load(&Sources {
            file: Some(PathBuf::from("/nonexistent/live0.toml")),
            ..Sources::default()
        })
        .unwrap_err();
        assert!(matches!(err, Error::Read { .. }), "{err:?}");
    }

    #[test]
    fn env_key_mapping() {
        assert_eq!(
            env_key("LIVE0_KAFKA__BROKERS").as_deref(),
            Some("kafka.brokers")
        );
        assert_eq!(
            env_key("LIVE0_TABLES__EVENTS__TOPIC").as_deref(),
            Some("tables.events.topic")
        );
        assert_eq!(env_key("LIVE0_E2E"), None);
        assert_eq!(env_key("LIVE0_KAFKA"), None);
        assert_eq!(env_key("OTHER_KAFKA__BROKERS"), None);
    }
}
