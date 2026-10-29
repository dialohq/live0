use serde_json::Value as Json;

use crate::data_types::{DataType, HistoryError};
use crate::value::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct ArrayType {
    pub(crate) item: Box<DataType>,
}

impl ArrayType {
    /// Each item decodes as a value of the item type would; a missing array is empty.
    pub(crate) fn decode_event(&self, json: Option<&Json>) -> Option<Vec<Value>> {
        match json {
            None | Some(Json::Null) => Some(Vec::new()),
            Some(Json::Array(items)) => items
                .iter()
                .map(|item| Value::from_event(Some(item), &self.item))
                .collect(),
            Some(_) => None,
        }
    }

    pub(crate) fn decode_history(&self, json: &Json) -> Result<Vec<Value>, HistoryError> {
        json.as_array()
            .ok_or_else(|| HistoryError::new("Array", json))?
            .iter()
            .map(|item| Value::from_history(item, &self.item))
            .collect()
    }

    /// `arrayMap` applies the item's expression when it needs one; `depth` keeps nested lambdas' names apart.
    pub(crate) fn history_expr(&self, column: &str, depth: usize) -> String {
        let x = format!("x{depth}");
        let item = self.item.history_expr_at(&x, depth + 1);
        match item == x {
            true => column.to_owned(),
            false => format!("arrayMap({x} -> {item}, {column})"),
        }
    }

    pub(crate) fn parse(raw: &str) -> Option<Self> {
        let inner = raw.strip_prefix("Array(")?.strip_suffix(')')?;
        Some(Self {
            item: Box::new(DataType::parse(inner)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_maps_items_only_when_needed() {
        assert_eq!(DataType::parse("Array(Int32)").history_expr("v"), "v");
        assert_eq!(
            DataType::parse("Array(Array(Date32))").history_expr("v"),
            "arrayMap(x0 -> arrayMap(x1 -> toInt32(x1), x0), v)"
        );
    }

    #[test]
    fn decoder_cases() {
        crate::data_types::cases::check(include_str!("../../tests/results/array.jsonl"));
    }
}
