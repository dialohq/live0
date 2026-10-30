use serde_json::Value as Json;

use crate::data_types::sink::java_order;
use crate::data_types::{DataType, HistoryError};
use crate::value::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct MapType {
    pub(crate) key: Box<DataType>,
    pub(crate) value: Box<DataType>,
}

impl MapType {
    /// A JSON object in the Sink's order, whose keys decode as the key type from their text; entries with a
    /// `null` value are dropped.
    pub(crate) fn decode_event(&self, json: Option<&Json>) -> Option<Vec<(Value, Value)>> {
        match json {
            None | Some(Json::Null) => Some(Vec::new()),
            Some(Json::Object(entries)) => java_order(entries)
                .into_iter()
                .filter(|(_, value)| !value.is_null())
                .map(|(key, value)| {
                    let key = Value::from_event(Some(&Json::String(key.clone())), &self.key)?;
                    Some((key, Value::from_event(Some(value), &self.value)?))
                })
                .collect(),
            Some(_) => None,
        }
    }

    pub(crate) fn decode_history(&self, json: &Json) -> Result<Vec<(Value, Value)>, HistoryError> {
        json.as_object()
            .ok_or_else(|| HistoryError::new("Map", json))?
            .iter()
            .map(|(key, value)| {
                Ok((
                    self.history_key(key)?,
                    Value::from_history(value, &self.value)?,
                ))
            })
            .collect()
    }

    /// JSON object keys are always text, so a number key is read back from its text.
    fn history_key(&self, key: &str) -> Result<Value, HistoryError> {
        Value::from_history(&Json::String(key.to_owned()), &self.key).or_else(|e| {
            match serde_json::from_str::<Json>(key) {
                Ok(json) if !json.is_string() => Value::from_history(&json, &self.key),
                _ => Err(e),
            }
        })
    }

    pub(crate) fn history_expr(&self, column: &str, depth: usize) -> String {
        let (k, v) = (format!("k{depth}"), format!("v{depth}"));
        let key = self.key.history_expr_at(&k, depth + 1);
        let value = self.value.history_expr_at(&v, depth + 1);
        match key == k && value == v {
            true => column.to_owned(),
            false => format!("mapApply(({k}, {v}) -> ({key}, {value}), {column})"),
        }
    }

    pub(crate) fn parse(raw: &str) -> Option<Self> {
        let args = raw.strip_prefix("Map(")?.strip_suffix(')')?;
        let (key, value) = split_top_level(args)?;
        Some(Self {
            key: Box::new(DataType::parse(key.trim())),
            value: Box::new(DataType::parse(value.trim())),
        })
    }
}

/// At the first comma outside parentheses and quotes.
fn split_top_level(s: &str) -> Option<(&str, &str)> {
    let (mut depth, mut quoted, mut escaped) = (0usize, false, false);
    for (i, c) in s.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if quoted => escaped = true,
            '\'' => quoted = !quoted,
            _ if quoted => {}
            '(' => depth += 1,
            ')' => depth = depth.checked_sub(1)?,
            ',' if depth == 0 => return Some((&s[..i], &s[i + 1..])),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_types() {
        let t = MapType::parse("Map(Enum8('a,b' = 1), Map(String, Array(Int32)))").unwrap();
        assert!(matches!(*t.key, DataType::Enum(_)));
        assert!(matches!(*t.value, DataType::Other(_) | DataType::Map(_)));
        assert_eq!(
            DataType::parse("Map(Date32, Array(Date32))").history_expr("v"),
            "mapApply((k0, v0) -> (toInt32(k0), arrayMap(x1 -> toInt32(x1), v0)), v)"
        );
        assert_eq!(DataType::parse("Map(String, Int32)").history_expr("v"), "v");
    }

    #[test]
    fn decoder_cases() {
        crate::data_types::cases::check(include_str!("../../tests/results/map.jsonl"));
    }
}
