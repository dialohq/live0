use std::time::SystemTime;

use async_trait::async_trait;
use types::Watermark;

use crate::Error;

/// A paused table's ingestion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pause {
    pub token: String,
    /// The next offset the sink would ingest, per partition of the table's topic.
    pub watermark: Watermark,
}

#[async_trait]
pub trait PauseServer: Send + Sync {
    async fn pause(&self, table: &str) -> Result<Pause, Error>;

    async fn resume(&self, pause: &Pause) -> Result<(), Error>;
}

/// A clone and what it was cloned at, kept with the clone so a restarted manager can adopt it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloneInfo {
    pub name: String,
    pub table: String,
    pub watermark: Watermark,
    pub taken_at: SystemTime,
}

#[async_trait]
pub trait Cloner: Send + Sync {
    /// Links the paused table's parts into a new table named `clone.name`, copying no data.
    async fn create(&self, clone: &CloneInfo) -> Result<(), Error>;

    async fn drop(&self, name: &str) -> Result<(), Error>;

    /// The clones under `namespace`, as created by this or an earlier manager.
    async fn list(&self, namespace: &str) -> Result<Vec<CloneInfo>, Error>;
}
