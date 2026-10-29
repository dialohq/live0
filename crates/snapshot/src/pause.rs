use async_trait::async_trait;

use crate::{Error, Pause, PauseServer};

/// Finds the leader among the replicas, retrying `503`s and connection failures for `retry_for`.
pub struct HttpPauseServer {}

impl HttpPauseServer {
    pub fn new(config: &config::PauseConfig) -> Self {
        todo!()
    }
}

#[async_trait]
impl PauseServer for HttpPauseServer {
    async fn pause(&self, table: &str) -> Result<Pause, Error> {
        todo!()
    }

    async fn resume(&self, pause: &Pause) -> Result<(), Error> {
        todo!()
    }
}
