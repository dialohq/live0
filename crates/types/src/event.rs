use crate::{Offset, Partition, Row};

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub topic: String,
    pub partition: Partition,
    pub offset: Offset,
    /// `None` when the record doesn't decode by the query's schema; its offset still counts.
    pub row: Option<Row>,
}
