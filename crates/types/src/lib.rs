pub use crate::data_types::{DataType, HistoryError};
pub use crate::event::Event;
pub use crate::row::{DecodeError, Row, decode_event, decode_history};
pub use crate::value::Value;
pub use crate::watermark::Watermark;

mod data_types;
mod event;
mod row;
mod value;
mod watermark;

pub type DecodeSpec = Vec<(String, DataType)>;

pub struct Snapshot {
    rows: Vec<Row>,
}

pub struct Insert {
    position: usize,
    row: Row,
}

pub enum Diff {
    Insert(Insert),
}
