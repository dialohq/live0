use crate::value::Value;

pub use crate::data_types::HistoryError;

mod data_types;
mod value;

pub const HISTORY_FORMAT: &str = "JSONEachRow";
pub const HISTORY_SETTINGS: &[(&str, &str)] = &[
    ("output_format_json_quote_64bit_integers", "1"),
    ("output_format_json_quote_denormals", "1"),
];

pub type Row = Vec<Value>;

pub struct Event {
    pub topic: String,
    pub partition: i32,
    pub offset: i64,
    pub value: Row,
}
