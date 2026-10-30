use std::{collections::BTreeMap, sync::Arc};

use config::{TableConfig, TablesConfig};
use io::{HistorySource, StreamSource};

use crate::{Table, build::build};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no table `{0}`")]
    UnknownTable(String),
    #[error("table `{table}`: {message}")]
    InvalidTable { table: String, message: String },
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// What can be queried; immutable, so a refresh or an on-demand load produces a new one.
#[derive(Debug, Clone)]
pub struct Catalog {
    config: TablesConfig,
    tables: BTreeMap<String, Arc<Table>>,
}

impl Catalog {
    /// Builds every configured table, checked against ClickHouse and Kafka.
    pub async fn load(
        config: &TablesConfig,
        history: &dyn HistorySource,
        stream: &dyn StreamSource,
    ) -> Result<Catalog, Error> {
        let mut tables = BTreeMap::new();
        for table in &config.tables {
            tables.insert(
                table.name.clone(),
                load_table(table, history, stream).await?,
            );
        }
        Ok(Catalog {
            config: config.clone(),
            tables,
        })
    }

    pub fn table(&self, name: &str) -> Option<&Arc<Table>> {
        self.tables.get(name)
    }

    pub fn tables(&self) -> impl Iterator<Item = &Arc<Table>> {
        self.tables.values()
    }

    /// The names a query needs that aren't here yet but could be loaded, in expose-all mode.
    pub fn missing<'a>(&self, names: &[&'a str]) -> Result<Vec<&'a str>, Error> {
        let mut missing: Vec<&'a str> = Vec::new();
        for &name in names {
            if self.tables.contains_key(name) || missing.contains(&name) {
                continue;
            }
            if !self.config.expose_all {
                return Err(Error::UnknownTable(name.to_owned()));
            }
            missing.push(name);
        }
        Ok(missing)
    }

    /// A catalog with these tables loaded as well; `self` is unchanged.
    pub async fn with_tables(
        &self,
        names: &[&str],
        history: &dyn HistorySource,
        stream: &dyn StreamSource,
    ) -> Result<Catalog, Error> {
        let mut catalog = self.clone();
        for &name in names {
            let config = match self.config.tables.iter().find(|t| t.name == name) {
                Some(config) => config.clone(),
                None if self.config.expose_all => TableConfig {
                    name: name.to_owned(),
                    topic: None,
                    grant: None,
                    columns: Vec::new(),
                    rows: Vec::new(),
                },
                None => return Err(Error::UnknownTable(name.to_owned())),
            };
            let table = load_table(&config, history, stream).await?;
            catalog.tables.insert(name.to_owned(), table);
        }
        Ok(catalog)
    }
}

async fn load_table(
    config: &TableConfig,
    history: &dyn HistorySource,
    stream: &dyn StreamSource,
) -> Result<Arc<Table>, Error> {
    let description = history
        .describe(&config.name)
        .await
        .map_err(|err| match err {
            io::Error::UnknownTable(table) => Error::UnknownTable(table),
            err => err.into(),
        })?;
    let topic = config.topic.as_deref().unwrap_or(&config.name);
    let partitions = stream.partitions(topic).await.map_err(|err| match err {
        io::Error::UnknownTopic(topic) => Error::InvalidTable {
            table: config.name.clone(),
            message: format!("topic `{topic}` doesn't exist"),
        },
        err => err.into(),
    })?;
    Ok(Arc::new(build(config, description, partitions)?))
}

#[cfg(test)]
mod tests {
    use io::{ColumnDefault, DescribedColumn, FakeCluster, TableDescription, TableEngine};
    use types::DataType;

    use super::*;

    fn description() -> TableDescription {
        TableDescription {
            columns: vec![DescribedColumn {
                name: "id".to_owned(),
                ty: DataType::parse("UInt64"),
                default: ColumnDefault::None,
            }],
            engine: TableEngine::MergeTree,
        }
    }

    fn table_config(name: &str, topic: Option<&str>) -> TableConfig {
        TableConfig {
            name: name.to_owned(),
            topic: topic.map(str::to_owned),
            grant: None,
            columns: Vec::new(),
            rows: Vec::new(),
        }
    }

    /// Tables `events` (topic `events`, 2 partitions) and `orders` (topic `raw_orders`), and `extra` unconfigured.
    fn cluster() -> FakeCluster {
        let cluster = FakeCluster::new();
        for (table, topic, partitions) in [
            ("events", "events", 2),
            ("orders", "raw_orders", 1),
            ("extra", "extra", 1),
        ] {
            cluster.create_topic(topic, partitions).unwrap();
            cluster.create_table(table, topic, description()).unwrap();
        }
        cluster
    }

    fn config(expose_all: bool) -> TablesConfig {
        TablesConfig {
            expose_all,
            tables: vec![
                table_config("events", None),
                table_config("orders", Some("raw_orders")),
            ],
        }
    }

    async fn load(cluster: &FakeCluster, config: &TablesConfig) -> Result<Catalog, Error> {
        Catalog::load(config, &cluster.history(), &cluster.stream()).await
    }

    #[tokio::test]
    async fn load_builds_every_configured_table() {
        let catalog = load(&cluster(), &config(false)).await.unwrap();
        let names: Vec<_> = catalog.tables().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["events", "orders"]);
        let events = catalog.table("events").unwrap();
        assert_eq!(
            (events.topic.as_str(), events.partitions.as_slice()),
            ("events", &[0, 1][..])
        );
        assert_eq!(catalog.table("orders").unwrap().topic, "raw_orders");
        assert!(catalog.table("extra").is_none());
    }

    #[tokio::test]
    async fn load_missing_table_is_unknown_table() {
        let mut config = config(false);
        config.tables.push(table_config("nope", Some("events")));
        let err = load(&cluster(), &config).await.unwrap_err();
        assert!(
            matches!(&err, Error::UnknownTable(t) if t == "nope"),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn load_missing_topic_is_invalid() {
        let mut config = config(false);
        config.tables[1].topic = Some("nope".to_owned());
        let err = load(&cluster(), &config).await.unwrap_err();
        assert!(
            matches!(&err, Error::InvalidTable { table, message }
                if table == "orders" && message.contains("topic `nope` doesn't exist")),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn missing_skips_loaded_and_dedups() {
        let catalog = load(&cluster(), &config(true)).await.unwrap();
        assert_eq!(
            catalog
                .missing(&["events", "extra", "other", "extra"])
                .unwrap(),
            ["extra", "other"]
        );
        assert_eq!(catalog.missing(&["orders"]).unwrap(), Vec::<&str>::new());
    }

    #[tokio::test]
    async fn missing_without_expose_all_is_unknown_table() {
        let catalog = load(&cluster(), &config(false)).await.unwrap();
        assert_eq!(catalog.missing(&["events"]).unwrap(), Vec::<&str>::new());
        let err = catalog.missing(&["events", "extra"]).unwrap_err();
        assert!(
            matches!(&err, Error::UnknownTable(t) if t == "extra"),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn with_tables_adds_and_keeps_self() {
        let cluster = cluster();
        let catalog = load(&cluster, &config(true)).await.unwrap();
        let wider = catalog
            .with_tables(&["extra"], &cluster.history(), &cluster.stream())
            .await
            .unwrap();
        let extra = wider.table("extra").unwrap();
        assert_eq!(extra.topic, "extra");
        assert_eq!(extra.security, crate::TableSecurity::default());
        assert!(wider.table("events").is_some());
        assert!(catalog.table("extra").is_none());
    }

    #[tokio::test]
    async fn with_tables_unknown_table_errors() {
        let cluster = cluster();
        let catalog = load(&cluster, &config(true)).await.unwrap();
        let err = catalog
            .with_tables(&["nope"], &cluster.history(), &cluster.stream())
            .await
            .unwrap_err();
        assert!(
            matches!(&err, Error::UnknownTable(t) if t == "nope"),
            "{err:?}"
        );
        let closed = load(&cluster, &config(false)).await.unwrap();
        let err = closed
            .with_tables(&["extra"], &cluster.history(), &cluster.stream())
            .await
            .unwrap_err();
        assert!(
            matches!(&err, Error::UnknownTable(t) if t == "extra"),
            "{err:?}"
        );
    }
}
