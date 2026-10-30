use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use async_trait::async_trait;
use io::{FakeCluster, FakeError};

use crate::{CloneInfo, Cloner, Error, Pause, PauseServer};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn table_error(table: &str, err: FakeError) -> Error {
    match err {
        FakeError::UnknownTable(_) => Error::UnknownTable(table.to_owned()),
        err => Error::Pause(err.to_string()),
    }
}

/// Pauses tables of a [`FakeCluster`]; like the real server, a table resumes when its last token does.
#[derive(Clone)]
pub struct FakePause {
    cluster: FakeCluster,
    failing: Arc<AtomicBool>,
    next: Arc<AtomicU64>,
    tokens: Arc<Mutex<BTreeMap<String, String>>>,
}

impl FakePause {
    pub fn new(cluster: FakeCluster) -> Self {
        Self {
            cluster,
            failing: Arc::default(),
            next: Arc::default(),
            tokens: Arc::default(),
        }
    }

    /// Makes later pauses fail until called with `false`.
    pub fn set_failing(&self, failing: bool) {
        self.failing.store(failing, Ordering::Relaxed);
    }

    /// How many pauses were handed out, so tests can tell a reused clone from a new one.
    pub fn pauses(&self) -> u64 {
        self.next.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl PauseServer for FakePause {
    async fn pause(&self, table: &str) -> Result<Pause, Error> {
        if self.failing.load(Ordering::Relaxed) {
            return Err(Error::Pause("pause server is failing".to_owned()));
        }
        let watermark = self
            .cluster
            .pause(table)
            .map_err(|err| table_error(table, err))?;
        let token = format!("fake-{}", self.next.fetch_add(1, Ordering::Relaxed));
        lock(&self.tokens).insert(token.clone(), table.to_owned());
        Ok(Pause { token, watermark })
    }

    async fn resume(&self, pause: &Pause) -> Result<(), Error> {
        let mut tokens = lock(&self.tokens);
        let table = tokens
            .remove(&pause.token)
            .ok_or_else(|| Error::Pause(format!("unknown token `{}`", pause.token)))?;
        if tokens.values().all(|held| *held != table) {
            self.cluster
                .resume(&table)
                .map_err(|err| table_error(&table, err))?;
        }
        Ok(())
    }
}

/// Clones tables inside the fake cluster, keeping each clone's info alongside.
#[derive(Clone)]
pub struct FakeCloner {
    cluster: FakeCluster,
    failing: Arc<AtomicBool>,
    infos: Arc<Mutex<BTreeMap<String, CloneInfo>>>,
}

impl FakeCloner {
    pub fn new(cluster: FakeCluster) -> Self {
        Self {
            cluster,
            failing: Arc::default(),
            infos: Arc::default(),
        }
    }

    /// Makes later clones fail until called with `false`.
    pub fn set_failing(&self, failing: bool) {
        self.failing.store(failing, Ordering::Relaxed);
    }
}

#[async_trait]
impl Cloner for FakeCloner {
    async fn create(&self, clone: &CloneInfo) -> Result<(), Error> {
        if self.failing.load(Ordering::Relaxed) {
            return Err(Error::Clone("cloner is failing".to_owned()));
        }
        self.cluster
            .clone_table(&clone.table, &clone.name)
            .map_err(|err| Error::Clone(err.to_string()))?;
        lock(&self.infos).insert(clone.name.clone(), clone.clone());
        Ok(())
    }

    async fn drop(&self, name: &str) -> Result<(), Error> {
        lock(&self.infos).remove(name);
        self.cluster
            .drop_table(name)
            .map_err(|err| Error::Clone(err.to_string()))
    }

    async fn list(&self, namespace: &str) -> Result<Vec<CloneInfo>, Error> {
        let prefix = format!("{namespace}__");
        Ok(lock(&self.infos)
            .values()
            .filter(|info| info.name.starts_with(&prefix))
            .cloned()
            .collect())
    }
}
