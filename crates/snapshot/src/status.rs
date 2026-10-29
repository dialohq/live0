use std::time::{Duration, SystemTime};

use types::Watermark;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub tables: Vec<TableStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableStatus {
    pub table: String,
    pub current: Option<CloneStatus>,
    /// Replaced clones still held by someone.
    pub retired: Vec<CloneStatus>,
    pub refreshing: bool,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloneStatus {
    pub name: String,
    pub watermark: Watermark,
    pub taken_at: SystemTime,
    /// How long pausing, cloning and resuming took.
    pub took: Duration,
    pub leases: usize,
}
