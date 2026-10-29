use ethnum::I256;
use serde_json::{Map, Value as Json};

use crate::data_types::{DataType, HistoryError};

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    String(String),
    Json(Map<String, Json>),
    Int(i128),
    Float(f64),
    Bool(bool),
    /// Days since the epoch.
    Date(i32),
    /// The number times `10^scale` of the column's type.
    Decimal(I256),
    /// Ticks of the column's precision since the epoch; the time zone is in the type.
    DateTime(i64),
}

impl Value {
    /// `json` is `None` when the event has no key for the column.
    pub fn from_event(json: Option<&Json>, ty: &DataType) -> Option<Value> {
        match ty {
            DataType::Nullable(_) if json.is_none_or(Json::is_null) => Some(Value::Null),
            DataType::Nullable(inner) => Value::from_event(json, inner),
            DataType::String(s) => s.decode_event(json).map(Value::String),
            DataType::Json(j) => j.decode_event(json).map(Value::Json),
            DataType::Int(i) => i.decode_event(json).map(Value::Int),
            DataType::Float(f) => f.decode_event(json).map(Value::Float),
            DataType::Bool(b) => b.decode_event(json).map(Value::Bool),
            DataType::Date(d) => d.decode_event(json).map(Value::Date),
            DataType::DateTime(t) => t.decode_event(json).map(Value::DateTime),
            DataType::Decimal(d) => d.decode_event(json).map(Value::Decimal),
            DataType::Other(_) => None,
        }
    }

    pub fn from_history(json: &Json, ty: &DataType) -> Result<Value, HistoryError> {
        match ty {
            DataType::Nullable(_) if json.is_null() => Ok(Value::Null),
            DataType::Nullable(inner) => Value::from_history(json, inner),
            DataType::String(s) => s.decode_history(json).map(Value::String),
            DataType::Json(j) => j.decode_history(json).map(Value::Json),
            DataType::Int(i) => i.decode_history(json).map(Value::Int),
            DataType::Float(f) => f.decode_history(json).map(Value::Float),
            DataType::Bool(b) => b.decode_history(json).map(Value::Bool),
            DataType::Date(d) => d.decode_history(json).map(Value::Date),
            DataType::DateTime(t) => t.decode_history(json).map(Value::DateTime),
            DataType::Decimal(d) => d.decode_history(json).map(Value::Decimal),
            DataType::Other(raw) => Err(HistoryError::new(raw, json)),
        }
    }
}
