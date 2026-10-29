use types::Watermark;

use crate::{Error, Lease, Status};

/// The gRPC client for a remote manager.
#[derive(Clone)]
pub(crate) struct Client {}

impl Client {
    pub(crate) async fn connect(config: &config::RemoteConfig) -> Result<Self, Error> {
        todo!()
    }

    /// A lease that renews itself in the background and sends a release when dropped.
    pub(crate) async fn acquire(
        &self,
        table: &str,
        newer_than: Option<&Watermark>,
    ) -> Result<Lease, Error> {
        todo!()
    }

    pub(crate) async fn status(&self) -> Result<Status, Error> {
        todo!()
    }
}
