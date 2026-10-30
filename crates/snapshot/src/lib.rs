pub use crate::cloner::ClickhouseCloner;
pub use crate::error::Error;
pub use crate::fake::{FakeCloner, FakePause};
pub use crate::handle::{Lease, Snapshots};
pub use crate::manager::Manager;
pub use crate::pause::HttpPauseServer;
pub use crate::ports::{CloneInfo, Cloner, Pause, PauseServer};
#[cfg(feature = "service")]
pub use crate::service::serve;
pub use crate::status::{CloneStatus, Status, TableStatus};

mod client;
mod cloner;
mod error;
mod fake;
mod handle;
mod manager;
mod pause;
mod ports;
#[cfg(feature = "service")]
mod service;
mod status;
#[cfg(test)]
mod tests;
