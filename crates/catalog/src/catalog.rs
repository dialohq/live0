use std::{collections::BTreeMap, sync::Arc};

use config::TablesConfig;
use io::{HistorySource, StreamSource};

use crate::Table;

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
        todo!()
    }

    pub fn table(&self, name: &str) -> Option<&Arc<Table>> {
        self.tables.get(name)
    }

    pub fn tables(&self) -> impl Iterator<Item = &Arc<Table>> {
        self.tables.values()
    }

    /// The names a query needs that aren't here yet but could be loaded, in expose-all mode.
    pub fn missing<'a>(&self, names: &[&'a str]) -> Result<Vec<&'a str>, Error> {
        todo!()
    }

    /// A catalog with these tables loaded as well; `self` is unchanged.
    pub async fn with_tables(
        &self,
        names: &[&str],
        history: &dyn HistorySource,
        stream: &dyn StreamSource,
    ) -> Result<Catalog, Error> {
        todo!()
    }
}
