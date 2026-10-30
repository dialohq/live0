use serde_json::Value as Json;

use crate::data_types::{DataType, HistoryError, split_args};
use crate::value::Value;

/// All elements are named or none are.
#[derive(Debug, Clone, PartialEq)]
pub struct TupleType {
    pub(crate) items: Vec<(Option<String>, DataType)>,
}

impl TupleType {
    fn named(&self) -> bool {
        self.items.iter().any(|(name, _)| name.is_some())
    }

    /// Named tuples take an object whose missing fields are defaults; unnamed ones an array of exactly their length.
    pub(crate) fn decode_event(&self, json: Option<&Json>) -> Option<Vec<Value>> {
        match json {
            None | Some(Json::Null) => self
                .items
                .iter()
                .map(|(_, ty)| Value::from_event(None, ty))
                .collect(),
            Some(Json::Object(fields)) if self.named() => self
                .items
                .iter()
                .map(|(name, ty)| Value::from_event(name.as_ref().and_then(|n| fields.get(n)), ty))
                .collect(),
            Some(Json::Array(values)) if !self.named() && values.len() == self.items.len() => {
                values
                    .iter()
                    .zip(&self.items)
                    .map(|(value, (_, ty))| Value::from_event(Some(value), ty))
                    .collect()
            }
            // ClickHouse reads the only element from no text, which works for integers, as 0 even if nullable.
            Some(Json::Array(values))
                if !self.named() && values.is_empty() && self.items.len() == 1 =>
            {
                let int = match &self.items[0].1 {
                    DataType::Nullable(inner) => inner.as_ref(),
                    ty => ty,
                };
                match int {
                    DataType::Int(i) => i.decode_event(None).map(|v| vec![v]),
                    _ => None,
                }
            }
            Some(_) => None,
        }
    }

    /// An object by name, or an array by position as [`Self::history_expr`] selects it.
    pub(crate) fn decode_history(&self, json: &Json) -> Result<Vec<Value>, HistoryError> {
        let error = || HistoryError::new("Tuple", json);
        match json {
            Json::Array(values) if values.len() == self.items.len() => values
                .iter()
                .zip(&self.items)
                .map(|(value, (_, ty))| Value::from_history(value, ty))
                .collect(),
            Json::Object(fields) => self
                .items
                .iter()
                .map(|(name, ty)| {
                    let value = name
                        .as_ref()
                        .and_then(|n| fields.get(n))
                        .ok_or_else(error)?;
                    Value::from_history(value, ty)
                })
                .collect(),
            _ => Err(error()),
        }
    }

    /// `tuple(...)` of the elements when any needs an expression; it drops the names, so history comes as an array.
    pub(crate) fn history_expr(&self, column: &str, depth: usize) -> String {
        let items: Vec<String> = (1..=self.items.len())
            .zip(&self.items)
            .map(|(i, (_, ty))| {
                ty.history_expr_at(&format!("tupleElement({column}, {i})"), depth + 1)
            })
            .collect();
        let plain =
            (1..=self.items.len()).all(|i| items[i - 1] == format!("tupleElement({column}, {i})"));
        match plain {
            true => column.to_owned(),
            false => format!("tuple({})", items.join(", ")),
        }
    }

    /// `Tuple(T, ...)` or `Tuple(name T, ...)`, names optionally in backticks.
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        let args = raw.strip_prefix("Tuple(")?.strip_suffix(')')?;
        let items: Vec<(Option<String>, DataType)> = split_args(args)?
            .into_iter()
            .map(|item| element(item.trim()))
            .collect();
        let named = items.iter().filter(|(name, _)| name.is_some()).count();
        (!items.is_empty() && (named == 0 || named == items.len())).then_some(Self { items })
    }
}

/// `name T`, `` `a name` T `` or a bare `T`; a space before any `(` separates a name.
fn element(item: &str) -> (Option<String>, DataType) {
    if let Some(rest) = item.strip_prefix('`')
        && let Some((name, ty)) = rest.split_once('`')
    {
        return (Some(name.to_owned()), DataType::parse(ty.trim()));
    }
    let space = item
        .find(' ')
        .filter(|s| item.find('(').is_none_or(|p| *s < p));
    match space {
        Some(s) => (
            Some(item[..s].to_owned()),
            DataType::parse(item[s..].trim()),
        ),
        None => (None, DataType::parse(item)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_types() {
        let t = TupleType::parse("Tuple(a Int32, `b c` DateTime64(3, 'UTC'))").unwrap();
        let names: Vec<_> = t.items.iter().map(|(n, _)| n.as_deref()).collect();
        assert_eq!(names, [Some("a"), Some("b c")]);
        assert!(matches!(t.items[1].1, DataType::DateTime(_)));
        assert!(
            TupleType::parse("Tuple(Int32, LowCardinality(String))").is_some_and(|t| !t.named())
        );
        assert_eq!(TupleType::parse("Tuple(a Int32, String)"), None);
        assert_eq!(
            DataType::parse("Tuple(a Date32, b String)").history_expr("v"),
            "tuple(toInt32(tupleElement(v, 1)), tupleElement(v, 2))"
        );
        assert_eq!(
            DataType::parse("Tuple(Int32, String)").history_expr("v"),
            "v"
        );
    }

    #[test]
    fn decoder_cases() {
        crate::data_types::cases::check(include_str!("../../tests/results/tuple.jsonl"));
    }
}
