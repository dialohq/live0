use async_trait::async_trait;
use io::FakeCluster;

use crate::{CloneInfo, Cloner, Error, Pause, PauseServer};

pub struct FakePause {
    cluster: FakeCluster,
}

impl FakePause {
    pub fn new(cluster: FakeCluster) -> Self {
        Self { cluster }
    }

    /// Makes later pauses fail until called with `false`.
    pub fn set_failing(&self, failing: bool) {
        todo!()
    }
}

#[async_trait]
impl PauseServer for FakePause {
    async fn pause(&self, table: &str) -> Result<Pause, Error> {
        todo!()
    }

    async fn resume(&self, pause: &Pause) -> Result<(), Error> {
        todo!()
    }
}

/// Clones tables inside the fake cluster, keeping each clone's info alongside.
pub struct FakeCloner {
    cluster: FakeCluster,
}

impl FakeCloner {
    pub fn new(cluster: FakeCluster) -> Self {
        Self { cluster }
    }

    /// Makes later clones fail until called with `false`.
    pub fn set_failing(&self, failing: bool) {
        todo!()
    }
}

#[async_trait]
impl Cloner for FakeCloner {
    async fn create(&self, clone: &CloneInfo) -> Result<(), Error> {
        todo!()
    }

    async fn drop(&self, name: &str) -> Result<(), Error> {
        todo!()
    }

    async fn list(&self, namespace: &str) -> Result<Vec<CloneInfo>, Error> {
        todo!()
    }
}
