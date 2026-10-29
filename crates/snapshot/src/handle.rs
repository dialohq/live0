use std::time::SystemTime;

use config::{ClickhouseConfig, PauseConfig, RemoteConfig, SnapshotsConfig};
use types::Watermark;

use crate::{Error, Manager, Status, client::Client};

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
    Embedded,
    Remote,
}

impl Snapshots {
    /// Runs a manager in-process or connects to a remote one, as the config says.
    pub async fn from_config(
        config: &SnapshotsConfig,
        clickhouse: &ClickhouseConfig,
        pause: &PauseConfig,
    ) -> Result<Self, Error> {
        todo!()
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
        todo!()
    }

    pub async fn status(&self) -> Result<Status, Error> {
        todo!()
    }
}

impl Lease {
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
