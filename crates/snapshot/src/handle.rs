use std::time::SystemTime;

use config::{ClickhouseConfig, PauseConfig, RemoteConfig, SnapshotsConfig};
use types::Watermark;

use crate::{CloneInfo, Error, Manager, Status, client::Client};

/// Table clones, the same whether the manager runs in this process or elsewhere.
#[derive(Clone)]
pub struct Snapshots {
    mode: Mode,
}

#[derive(Clone)]
enum Mode {
    Embedded(Manager),
    Remote(Client),
}

/// A held clone; renewed while held, released when dropped.
pub struct Lease {
    table: String,
    clone: String,
    watermark: Watermark,
    taken_at: SystemTime,
    holder: Holder,
}

/// Dropping it releases the clone.
enum Holder {
    Embedded { manager: Manager, id: u64 },
    Remote,
}

impl Snapshots {
    /// Runs a manager in-process or connects to a remote one, as the config says.
    pub async fn from_config(
        config: &SnapshotsConfig,
        clickhouse: &ClickhouseConfig,
        pause: &PauseConfig,
    ) -> Result<Self, Error> {
        match config {
            SnapshotsConfig::Embedded(manager) => Ok(Self::embedded(
                Manager::from_config(manager, clickhouse, pause).await?,
            )),
            SnapshotsConfig::Remote(remote) => Self::remote(remote).await,
        }
    }

    pub fn embedded(manager: Manager) -> Self {
        Self {
            mode: Mode::Embedded(manager),
        }
    }

    pub async fn remote(config: &RemoteConfig) -> Result<Self, Error> {
        todo!()
    }

    /// The table's current clone, or with `newer_than` one taken past that watermark.
    pub async fn acquire(
        &self,
        table: &str,
        newer_than: Option<&Watermark>,
    ) -> Result<Lease, Error> {
        match &self.mode {
            Mode::Embedded(manager) => manager.acquire_local(table, newer_than).await,
            Mode::Remote(client) => client.acquire(table, newer_than).await,
        }
    }

    pub async fn status(&self) -> Result<Status, Error> {
        match &self.mode {
            Mode::Embedded(manager) => Ok(manager.status().await),
            Mode::Remote(client) => client.status().await,
        }
    }
}

impl Lease {
    pub(crate) fn embedded(manager: Manager, id: u64, info: &CloneInfo) -> Self {
        Self {
            table: info.table.clone(),
            clone: info.name.clone(),
            watermark: info.watermark.clone(),
            taken_at: info.taken_at,
            holder: Holder::Embedded { manager, id },
        }
    }

    pub fn table(&self) -> &str {
        &self.table
    }

    /// The clone's table name, to run history queries against.
    pub fn clone_name(&self) -> &str {
        &self.clone
    }

    /// Exactly the offsets below it are in the clone; the stream starts here.
    pub fn watermark(&self) -> &Watermark {
        &self.watermark
    }

    pub fn taken_at(&self) -> SystemTime {
        self.taken_at
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        match &self.holder {
            Holder::Embedded { manager, id } => manager.release_local(*id),
            Holder::Remote => {}
        }
    }
}
