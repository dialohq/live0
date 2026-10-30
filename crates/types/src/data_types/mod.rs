use serde_json::Value as Json;

pub(crate) use crate::data_types::sink::strip_leading_zeros;

use crate::data_types::{
    array::ArrayType,
    boolean::BoolType,
    date::DateType,
    datetime::DateTimeType,
    decimal::DecimalType,
    enumeration::EnumType,
    fixed_string::FixedStringType,
    float::FloatType,
    int::IntType,
    ip::{Ipv4Type, Ipv6Type},
    json::JsonType,
    map::MapType,
    string::StringType,
    tuple::TupleType,
    uuid::UuidType,
};

mod array;
mod boolean;
#[cfg(test)]
mod cases;
mod date;
mod datetime;
mod decimal;
mod enumeration;
mod fixed_string;
mod float;
mod int;
mod ip;
mod json;
mod map;
mod sink;
mod string;
mod tuple;
mod uuid;

/// A history value that doesn't fit its column, so the history query or ClickHouse doesn't behave as expected.
#[derive(Debug, thiserror::Error)]
#[error("history value {value} is not a valid {ty}")]
pub struct HistoryError {
    pub ty: String,
    pub value: Json,
}

impl HistoryError {
    pub(crate) fn new(ty: impl Into<String>, value: &Json) -> Self {
        Self {
            ty: ty.into(),
            value: value.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DataType {
    Nullable(Box<DataType>),
    String(StringType),
    Json(JsonType),
    Int(IntType),
    Float(FloatType),
    Bool(BoolType),
    Date(DateType),
    DateTime(DateTimeType),
    Decimal(DecimalType),
    Uuid(UuidType),
    Enum(EnumType),
    FixedString(FixedStringType),
    Ipv4(Ipv4Type),
    Ipv6(Ipv6Type),
    Array(ArrayType),
    Map(MapType),
    Tuple(TupleType),
    Other(String),
}

impl DataType {
    pub fn parse(raw: &str) -> Self {
        let parsed = if let Some(inner) = unwrap(raw, "LowCardinality") {
            Self::parse(inner)
        } else if let Some(inner) = unwrap(raw, "Nullable") {
            Self::Nullable(Box::new(Self::parse(inner)))
        } else {
            Self::parse_plain(raw)
        };
        if parsed.is_supported() {
            parsed
        } else {
            Self::Other(raw.to_owned())
        }
    }

    fn parse_plain(raw: &str) -> Self {
        match raw {
            "String" => Self::String(StringType),
            "JSON" => Self::Json(JsonType),
            "Int8" => Self::int(true, 8),
            "Int16" => Self::int(true, 16),
            "Int32" => Self::int(true, 32),
            "Int64" => Self::int(true, 64),
            "UInt8" => Self::int(false, 8),
            "UInt16" => Self::int(false, 16),
            "UInt32" => Self::int(false, 32),
            "UInt64" => Self::int(false, 64),
            "Int128" => Self::int(true, 128),
            "UInt128" => Self::int(false, 128),
            "Int256" => Self::int(true, 256),
            "UInt256" => Self::int(false, 256),
            "Float32" => Self::Float(FloatType { width: 32 }),
            "Float64" => Self::Float(FloatType { width: 64 }),
            "Bool" => Self::Bool(BoolType),
            "Date" => Self::Date(DateType { wide: false }),
            "Date32" => Self::Date(DateType { wide: true }),
            "UUID" => Self::Uuid(UuidType),
            "IPv4" => Self::Ipv4(Ipv4Type),
            "IPv6" => Self::Ipv6(Ipv6Type),
            _ => DateTimeType::parse(raw)
                .map(Self::DateTime)
                .or_else(|| DecimalType::parse_type(raw).map(Self::Decimal))
                .or_else(|| EnumType::parse(raw).map(Self::Enum))
                .or_else(|| FixedStringType::parse(raw).map(Self::FixedString))
                .or_else(|| ArrayType::parse(raw).map(Self::Array))
                .or_else(|| MapType::parse(raw).map(Self::Map))
                .or_else(|| TupleType::parse(raw).map(Self::Tuple))
                .unwrap_or_else(|| Self::Other(raw.to_owned())),
        }
    }

    /// How a history query selects a column of this type so that [`crate::Value::from_history`] reads it exactly.
    pub fn history_expr(&self, column: &str) -> String {
        self.history_expr_at(column, 0)
    }

    pub(crate) fn history_expr_at(&self, column: &str, depth: usize) -> String {
        match self {
            Self::Nullable(inner) => inner.history_expr_at(column, depth),
            Self::Date(d) => d.history_expr(column),
            Self::Array(a) => a.history_expr(column, depth),
            Self::Map(m) => m.history_expr(column, depth),
            Self::Tuple(t) => t.history_expr(column, depth),
            _ => column.to_owned(),
        }
    }

    fn int(signed: bool, width: u16) -> Self {
        Self::Int(IntType { signed, width })
    }

    fn is_supported(&self) -> bool {
        match self {
            Self::Other(_) => false,
            Self::Nullable(inner) => inner.is_supported(),
            Self::Array(a) => a.item.is_supported(),
            Self::Map(m) => m.key.is_supported() && m.value.is_supported(),
            Self::Tuple(t) => t.items.iter().all(|(_, ty)| ty.is_supported()),
            _ => true,
        }
    }
}

/// A type's arguments, split at the commas outside parentheses and quotes.
pub(crate) fn split_args(s: &str) -> Option<Vec<&str>> {
    let (mut depth, mut quoted, mut escaped) = (0usize, false, false);
    let (mut args, mut start) = (Vec::new(), 0);
    for (i, c) in s.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if quoted => escaped = true,
            '\'' => quoted = !quoted,
            _ if quoted => {}
            '(' => depth += 1,
            ')' => depth = depth.checked_sub(1)?,
            ',' if depth == 0 => {
                args.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    args.push(&s[start..]);
    Some(args)
}

fn unwrap<'a>(raw: &'a str, wrapper: &str) -> Option<&'a str> {
    raw.strip_prefix(wrapper)?
        .strip_prefix('(')?
        .strip_suffix(')')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_types() {
        assert_eq!(DataType::parse("String"), DataType::String(StringType));
        assert_eq!(DataType::parse("UInt16"), DataType::int(false, 16));
        assert_eq!(DataType::parse("Int64"), DataType::int(true, 64));
        assert_eq!(
            DataType::parse("Float32"),
            DataType::Float(FloatType { width: 32 })
        );
    }

    #[test]
    fn low_cardinality_is_transparent() {
        assert_eq!(
            DataType::parse("LowCardinality(String)"),
            DataType::String(StringType)
        );
    }

    #[test]
    fn nullable() {
        assert_eq!(
            DataType::parse("Nullable(Int32)"),
            DataType::Nullable(Box::new(DataType::int(true, 32)))
        );
        assert_eq!(
            DataType::parse("LowCardinality(Nullable(String))"),
            DataType::Nullable(Box::new(DataType::String(StringType)))
        );
    }

    #[test]
    fn unsupported_keeps_the_full_name() {
        for raw in [
            "Point",
            "LowCardinality(Polygon)",
            "Nullable(Ring)",
            "LowCardinality(Nullable(MultiPolygon))",
            "Nullable(String",
            "LowCardinality",
        ] {
            assert_eq!(
                DataType::parse(raw),
                DataType::Other(raw.to_owned()),
                "{raw}"
            );
        }
    }
}
