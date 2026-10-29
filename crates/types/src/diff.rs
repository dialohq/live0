use crate::Row;

/// A whole result, in the order it is shown.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub rows: Vec<Row>,
}

/// A change to the shown result; positions are valid only after every earlier diff is applied.
#[derive(Debug, Clone, PartialEq)]
pub enum Diff {
    Insert { position: usize, row: Row },
    Update { position: usize, row: Row },
    Delete { position: usize },
}
