use serde_json::Value as Json;

use crate::{DecodeSpec, HistoryError, Value};

pub type Row = Vec<Value>;

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("history row is neither an array nor an object: {0}")]
    NotARow(Json),
    #[error("history row has {got} values for {expected} columns")]
    Length { got: usize, expected: usize },
    #[error("history row has no `{0}` column")]
    MissingColumn(String),
    #[error("column `{column}`: {source}")]
    Value {
        column: String,
        source: HistoryError,
    },
}

pub fn decode_event(json: &Json, spec: &DecodeSpec) -> Option<Row> {
    let object = json.as_object()?;
    spec.iter()
        .map(|column| Value::from_event(object.get(&column.0), &column.1))
        .collect()
}

pub fn decode_history(json: &Json, spec: &DecodeSpec) -> Result<Row, DecodeError> {
    let values: Vec<&Json> = match json {
        Json::Array(values) if values.len() == spec.len() => values.iter().collect(),
        Json::Array(values) => {
            return Err(DecodeError::Length {
                got: values.len(),
                expected: spec.len(),
            });
        }
        Json::Object(object) => spec
            .iter()
            .map(|column| {
                object
                    .get(&column.0)
                    .ok_or_else(|| DecodeError::MissingColumn(column.0.clone()))
            })
            .collect::<Result<_, _>>()?,
        other => return Err(DecodeError::NotARow(other.clone())),
    };
    values
        .into_iter()
        .zip(spec)
        .map(|(json, column)| {
            Value::from_history(json, &column.1).map_err(|source| DecodeError::Value {
                column: column.0.clone(),
                source,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::DataType;

    use super::*;

    fn spec() -> DecodeSpec {
        [("id", "UInt64"), ("name", "Nullable(String)")]
            .into_iter()
            .map(|(name, ty)| (name.to_owned(), DataType::parse(ty)))
            .collect()
    }

    #[test]
    fn event_columns_by_name() {
        assert_eq!(
            decode_event(&json!({"name": "a", "id": 1, "extra": true}), &spec()),
            Some(vec![Value::Int(1), Value::String("a".to_owned())])
        );
        assert_eq!(
            decode_event(&json!({}), &spec()),
            Some(vec![Value::Int(0), Value::Null])
        );
    }

    #[test]
    fn rejected_event() {
        assert_eq!(decode_event(&json!({"id": -1}), &spec()), None);
        assert_eq!(decode_event(&json!([1, "a"]), &spec()), None);
    }

    #[test]
    fn history_array_and_object() {
        let row = vec![Value::Int(1), Value::Null];
        assert_eq!(decode_history(&json!(["1", null]), &spec()).unwrap(), row);
        assert_eq!(
            decode_history(&json!({"name": null, "id": "1"}), &spec()).unwrap(),
            row
        );
    }

    #[test]
    fn history_errors() {
        assert!(matches!(
            decode_history(&json!(["1"]), &spec()),
            Err(DecodeError::Length {
                got: 1,
                expected: 2
            })
        ));
        assert!(matches!(
            decode_history(&json!({"id": "1"}), &spec()),
            Err(DecodeError::MissingColumn(c)) if c == "name"
        ));
        assert!(matches!(
            decode_history(&json!(["x", null]), &spec()),
            Err(DecodeError::Value { column, .. }) if column == "id"
        ));
    }
}
