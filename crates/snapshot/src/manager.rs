use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, MutexGuard, PoisonError, Weak},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use config::{ClickhouseConfig, ManagerConfig, PauseConfig};
use io::ClickhouseClient;
use tokio::{sync::Notify, time::Instant};
use types::Watermark;

use crate::{
    ClickhouseCloner, CloneInfo, CloneStatus, Cloner, Error, HttpPauseServer, Lease, PauseServer,
    Status, TableStatus,
};

/// How long to wait before pausing again when a fresh clone still isn't past `newer_than`.
const RETAKE_DELAY: Duration = Duration::from_millis(250);

/// Keeps a current, shared clone per table in use; the same logic serves embedded and remote holders.
#[derive(Clone)]
pub struct Manager {
    inner: Arc<Inner>,
}

struct Inner {
    config: ManagerConfig,
    pause: Arc<dyn PauseServer>,
    cloner: Arc<dyn Cloner>,
    state: Mutex<State>,
    /// Woken whenever a clone is installed or a refresh ends.
    changed: Notify,
}

#[derive(Default)]
struct State {
    tables: BTreeMap<String, TableState>,
    /// Each local lease's table and clone.
    leases: BTreeMap<u64, (String, String)>,
    next_lease: u64,
    next_clone: u64,
}

#[derive(Default)]
struct TableState {
    current: Option<Entry>,
    /// Replaced or adopted clones, dropped once nobody holds them.
    retired: Vec<Entry>,
    refreshing: bool,
    last_error: Option<String>,
}

struct Entry {
    info: CloneInfo,
    took: Duration,
    leases: usize,
    /// Adopted clones wait this long for holders from before the restart.
    keep_until: Option<Instant>,
}

impl Entry {
    fn status(&self) -> CloneStatus {
        CloneStatus {
            name: self.info.name.clone(),
            watermark: self.info.watermark.clone(),
            taken_at: self.info.taken_at,
            took: self.took,
            leases: self.leases,
        }
    }

    fn droppable(&self, now: Instant) -> bool {
        self.leases == 0 && self.keep_until.is_none_or(|until| now >= until)
    }
}

/// A clone handed to a remote holder, which expires unless renewed.
pub(crate) struct RemoteGrant {}

impl Manager {
    /// Adopts the existing clones in the namespace and starts refreshing.
    pub async fn start(
        config: &ManagerConfig,
        pause: Arc<dyn PauseServer>,
        cloner: Arc<dyn Cloner>,
    ) -> Result<Manager, Error> {
        let keep_until = Instant::now() + config.adopt_grace;
        let mut state = State::default();
        for info in cloner.list(&config.namespace).await? {
            state
                .tables
                .entry(info.table.clone())
                .or_default()
                .retired
                .push(Entry {
                    info,
                    took: Duration::ZERO,
                    leases: 0,
                    keep_until: Some(keep_until),
                });
        }
        let inner = Arc::new(Inner {
            config: config.clone(),
            pause,
            cloner,
            state: Mutex::new(state),
            changed: Notify::new(),
        });
        spawn_refresh(Arc::downgrade(&inner), config.refresh_interval);
        Ok(Manager { inner })
    }

    /// With the real pause-server client and cloner.
    pub async fn from_config(
        config: &ManagerConfig,
        clickhouse: &ClickhouseConfig,
        pause: &PauseConfig,
    ) -> Result<Manager, Error> {
        let pause = Arc::new(HttpPauseServer::new(pause));
        let cloner = Arc::new(ClickhouseCloner::new(ClickhouseClient::new(clickhouse)));
        Self::start(config, pause, cloner).await
    }

    pub async fn status(&self) -> Status {
        let state = self.state();
        Status {
            tables: state
                .tables
                .iter()
                .map(|(table, t)| TableStatus {
                    table: table.clone(),
                    current: t.current.as_ref().map(Entry::status),
                    retired: t.retired.iter().map(Entry::status).collect(),
                    refreshing: t.refreshing,
                    last_error: t.last_error.clone(),
                })
                .collect(),
        }
    }

    /// An in-process lease, held until dropped.
    pub(crate) async fn acquire_local(
        &self,
        table: &str,
        newer_than: Option<&Watermark>,
    ) -> Result<Lease, Error> {
        let mut took = false;
        loop {
            let notified = self.inner.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let take = {
                let mut state = self.state();
                let State {
                    tables,
                    leases,
                    next_lease,
                    ..
                } = &mut *state;
                let entry = tables.entry(table.to_owned()).or_default();
                if let Some(current) = entry
                    .current
                    .as_mut()
                    .filter(|c| newer_than.is_none_or(|w| c.info.watermark.covers(w)))
                {
                    current.leases += 1;
                    let id = *next_lease;
                    *next_lease += 1;
                    leases.insert(id, (table.to_owned(), current.info.name.clone()));
                    return Ok(Lease::embedded(self.clone(), id, &current.info));
                }
                !std::mem::replace(&mut entry.refreshing, true)
            };
            if !take {
                notified.await;
                continue;
            }
            if took {
                tokio::time::sleep(RETAKE_DELAY).await;
            }
            self.refresh(table).await?;
            took = true;
        }
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

    /// Lets go of a local lease; a retired clone nobody holds is dropped on the next refresh.
    pub(crate) fn release_local(&self, id: u64) {
        let mut state = self.state();
        let State { tables, leases, .. } = &mut *state;
        let Some((table, clone)) = leases.remove(&id) else {
            return;
        };
        if let Some(t) = tables.get_mut(&table) {
            for entry in t.current.iter_mut().chain(&mut t.retired) {
                if entry.info.name == clone {
                    entry.leases = entry.leases.saturating_sub(1);
                }
            }
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Takes a new current clone of a table whose `refreshing` the caller set.
    async fn refresh(&self, table: &str) -> Result<(), Error> {
        let taken = self.take(table).await;
        let result = {
            let mut state = self.state();
            let entry = state.tables.entry(table.to_owned()).or_default();
            entry.refreshing = false;
            match taken {
                Ok((new, resume_error)) => {
                    entry.last_error = resume_error;
                    entry.retired.extend(entry.current.replace(new));
                    Ok(())
                }
                Err(err) => {
                    entry.last_error = Some(err.to_string());
                    Err(err)
                }
            }
        };
        self.inner.changed.notify_waiters();
        self.collect().await;
        result
    }

    /// Pauses, clones and always resumes; a failed resume is reported but leaves a good clone.
    async fn take(&self, table: &str) -> Result<(Entry, Option<String>), Error> {
        let started = Instant::now();
        let pause = self.inner.pause.pause(table).await?;
        let info = CloneInfo {
            name: self.clone_name(table),
            table: table.to_owned(),
            watermark: pause.watermark.clone(),
            taken_at: SystemTime::now(),
        };
        let created = self.inner.cloner.create(&info).await;
        let resumed = self.inner.pause.resume(&pause).await;
        created?;
        let entry = Entry {
            info,
            took: started.elapsed(),
            leases: 0,
            keep_until: None,
        };
        Ok((entry, resumed.err().map(|err| err.to_string())))
    }

    fn clone_name(&self, table: &str) -> String {
        let seq = {
            let mut state = self.state();
            state.next_clone += 1;
            state.next_clone
        };
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        format!("{}__{table}__{millis}_{seq}", self.inner.config.namespace)
    }

    /// Refreshes tables in use, retires idle ones' clones and drops what nobody holds.
    async fn tick(&self) {
        let refresh: Vec<String> = {
            let mut state = self.state();
            let mut refresh = Vec::new();
            for (table, t) in &mut state.tables {
                if t.refreshing {
                    continue;
                }
                match t.current.as_ref().map(|c| c.leases) {
                    Some(0) => t.retired.extend(t.current.take()),
                    Some(_) => {
                        t.refreshing = true;
                        refresh.push(table.clone());
                    }
                    None => {}
                }
            }
            refresh
        };
        for table in refresh {
            let _ = self.refresh(&table).await;
        }
        self.collect().await;
    }

    /// Drops retired clones nobody holds, and forgets tables left with nothing.
    async fn collect(&self) {
        let now = Instant::now();
        let doomed: Vec<(String, String)> = {
            let mut state = self.state();
            let mut doomed = Vec::new();
            for (table, t) in &mut state.tables {
                t.retired.retain(|entry| {
                    let drop = entry.droppable(now);
                    if drop {
                        doomed.push((table.clone(), entry.info.name.clone()));
                    }
                    !drop
                });
            }
            doomed
        };
        for (table, clone) in doomed {
            if let Err(err) = Cloner::drop(self.inner.cloner.as_ref(), &clone).await
                && let Some(t) = self.state().tables.get_mut(&table)
            {
                t.last_error = Some(err.to_string());
            }
        }
        self.state().tables.retain(|_, t| {
            t.current.is_some() || !t.retired.is_empty() || t.refreshing || t.last_error.is_some()
        });
    }
}

/// Ticks every `interval` until the manager is dropped.
fn spawn_refresh(inner: Weak<Inner>, interval: Duration) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval_at(Instant::now() + interval, interval);
        loop {
            ticker.tick().await;
            let Some(inner) = inner.upgrade() else {
                break;
            };
            Manager { inner }.tick().await;
        }
    });
}
