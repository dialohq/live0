use async_trait::async_trait;
use io::ClickhouseClient;

use crate::{CloneInfo, Cloner, Error};

/// `CREATE TABLE … AS` plus `ATTACH PARTITION ALL FROM`, with the clone's info in its table comment.
pub struct ClickhouseCloner {
    client: ClickhouseClient,
}

impl ClickhouseCloner {
    pub fn new(client: ClickhouseClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Cloner for ClickhouseCloner {
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
