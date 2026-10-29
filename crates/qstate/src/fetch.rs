use compiler::HistorySql;
use types::{Row, Watermark};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FetchId(pub u64);

/// A history query the state needs answered.
#[derive(Debug, Clone, PartialEq)]
pub struct Fetch {
    pub id: FetchId,
    /// With its slots filled; the tables are still to be bound to clones.
    pub sql: HistorySql,
    pub freshness: Freshness,
}

/// Which clone a fetch may run against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Freshness {
    /// The current one, however old.
    Any,
    /// One taken past this watermark, e.g. to refill a window from rows the stream hasn't reached.
    NewerThan(Watermark),
    /// The one the subscription holds for its lifetime, as semi-joins need.
    Held,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub id: FetchId,
    pub rows: Vec<Row>,
    /// The watermark of the clone the fetch ran against.
    pub watermark: Watermark,
}
