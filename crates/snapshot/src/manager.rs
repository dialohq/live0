use std::sync::Arc;

use config::{ClickhouseConfig, ManagerConfig, PauseConfig};
use types::Watermark;

use crate::{Cloner, Error, PauseServer, Status};

/// Keeps a current, shared clone per table in use; the same logic serves embedded and remote holders.
#[derive(Clone)]
pub struct Manager {
    inner: Arc<Inner>,
}

struct Inner {}

/// A clone handed to a remote holder, which expires unless renewed.
pub(crate) struct RemoteGrant {}

impl Manager {
    /// Adopts the existing clones in the namespace and starts refreshing.
    pub async fn start(
        config: &ManagerConfig,
        pause: Arc<dyn PauseServer>,
        cloner: Arc<dyn Cloner>,
    ) -> Result<Manager, Error> {
        todo!()
    }

    /// With the real pause-server client and cloner.
    pub async fn from_config(
        config: &ManagerConfig,
        clickhouse: &ClickhouseConfig,
        pause: &PauseConfig,
    ) -> Result<Manager, Error> {
        todo!()
    }

    pub async fn status(&self) -> Status {
        todo!()
    }

    /// An in-process lease, held until dropped.
    pub(crate) async fn acquire_local(
        &self,
        table: &str,
        newer_than: Option<&Watermark>,
    ) -> Result<crate::Lease, Error> {
        todo!()
    }

    pub(crate) async fn acquire_remote(
        &self,
        holder: &str,
        table: &str,
        newer_than: Option<&Watermark>,
    ) -> Result<RemoteGrant, Error> {
        todo!()
    }

    pub(crate) async fn renew(&self, lease_id: &str) -> Result<(), Error> {
        todo!()
    }

    pub(crate) async fn release(&self, lease_id: &str) {
        todo!()
    }
}
