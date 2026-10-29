use std::sync::{Arc, PoisonError, RwLock};

use catalog::Catalog;
use compiler::{Access, Params, Plan};
use config::{Config, TablesConfig};
use io::{ClickhouseClient, ClickhouseHistory, HistorySource, KafkaStream, StreamSource};
use qstate::Checkpoint;
use snapshot::Snapshots;

use crate::{Error, Subscription};

#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub sql: String,
    pub params: Params,
    pub access: Access,
}

/// One long-lived engine serving many subscriptions.
#[derive(Clone)]
pub struct Engine {
    inner: Arc<Inner>,
}

pub(crate) struct Inner {
    pub(crate) history: Arc<dyn HistorySource>,
    pub(crate) stream: Arc<dyn StreamSource>,
    pub(crate) snapshots: Snapshots,
    /// Replaced, never mutated, when tables are loaded on demand.
    catalog: RwLock<Arc<Catalog>>,
}

impl Engine {
    /// Loads the catalog through the ports.
    pub async fn new(
        history: Arc<dyn HistorySource>,
        stream: Arc<dyn StreamSource>,
        snapshots: Snapshots,
        tables: &TablesConfig,
    ) -> Result<Engine, Error> {
        let catalog = Catalog::load(tables, history.as_ref(), stream.as_ref()).await?;
        Ok(Engine {
            inner: Arc::new(Inner {
                history,
                stream,
                snapshots,
                catalog: RwLock::new(Arc::new(catalog)),
            }),
        })
    }

    /// With ClickHouse, Kafka and snapshots as configured.
    pub async fn from_config(config: &Config) -> Result<Engine, Error> {
        let clickhouse = ClickhouseClient::new(&config.clickhouse);
        let history = Arc::new(ClickhouseHistory::new(clickhouse));
        let stream = Arc::new(KafkaStream::new(&config.kafka).map_err(io::Error::from)?);
        let snapshots =
            Snapshots::from_config(&config.snapshots, &config.clickhouse, &config.pause).await?;
        Self::new(history, stream, snapshots, &config.tables).await
    }

    /// Compiles the query without running it.
    pub async fn explain(&self, query: &Query) -> Result<Plan, Error> {
        let parsed = parser::parse(&query.sql)?;
        let catalog = self.catalog(&parsed.tables()).await?;
        Ok(compiler::compile(
            &parsed,
            &catalog,
            &query.params,
            &query.access,
        )?)
    }

    /// Fails on queries that don't compile; history and later errors come from the subscription.
    pub async fn subscribe(
        &self,
        query: &Query,
        resume: Option<Checkpoint>,
    ) -> Result<Subscription, Error> {
        let plan = Arc::new(self.explain(query).await?);
        let state = qstate::start(plan.clone(), resume)?;
        Ok(Subscription::new(self.inner.clone(), plan, state))
    }

    /// A catalog with every table in `tables`, loading the missing ones.
    async fn catalog(&self, tables: &[&str]) -> Result<Arc<Catalog>, Error> {
        let current = self
            .inner
            .catalog
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let missing = current.missing(tables)?;
        if missing.is_empty() {
            return Ok(current);
        }
        let loaded = Arc::new(
            current
                .with_tables(
                    &missing,
                    self.inner.history.as_ref(),
                    self.inner.stream.as_ref(),
                )
                .await?,
        );
        *self
            .inner
            .catalog
            .write()
            .unwrap_or_else(PoisonError::into_inner) = loaded.clone();
        Ok(loaded)
    }
}
