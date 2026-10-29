use serde_json::Value as Json;

use crate::{HistoryError, Schema, Value};

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

pub fn decode_event(json: &Json, schema: &Schema) -> Option<Row> {
    let object = json.as_object()?;
    schema
        .columns
        .iter()
        .map(|column| Value::from_event(object.get(&column.name), &column.ty))
        .collect()
}

pub fn decode_history(json: &Json, schema: &Schema) -> Result<Row, DecodeError> {
    let values: Vec<&Json> = match json {
        Json::Array(values) if values.len() == schema.columns.len() => values.iter().collect(),
        Json::Array(values) => {
            return Err(DecodeError::Length {
                got: values.len(),
                expected: schema.columns.len(),
            });
        }
        Json::Object(object) => schema
            .columns
            .iter()
            .map(|column| {
                object
                    .get(&column.name)
                    .ok_or_else(|| DecodeError::MissingColumn(column.name.clone()))
            })
            .collect::<Result<_, _>>()?,
        other => return Err(DecodeError::NotARow(other.clone())),
    };
    values
        .into_iter()
        .zip(&schema.columns)
        .map(|(json, column)| {
            Value::from_history(json, &column.ty).map_err(|source| DecodeError::Value {
                column: column.name.clone(),
                source,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{Column, DataType};

    use super::*;

    fn schema() -> Schema {
        Schema {
            columns: [("id", "UInt64"), ("name", "Nullable(String)")]
                .into_iter()
                .map(|(name, ty)| Column::new(name, DataType::parse(ty)))
                .collect(),
        }
    }

    #[test]
    fn event_columns_by_name() {
        assert_eq!(
            decode_event(&json!({"name": "a", "id": 1, "extra": true}), &schema()),
            Some(vec![Value::Int(1), Value::String("a".to_owned())])
        );
        assert_eq!(
            decode_event(&json!({}), &schema()),
            Some(vec![Value::Int(0), Value::Null])
        );
    }

    #[test]
    fn rejected_event() {
        assert_eq!(decode_event(&json!({"id": -1}), &schema()), None);
        assert_eq!(decode_event(&json!([1, "a"]), &schema()), None);
    }

    #[test]
    fn history_array_and_object() {
        let row = vec![Value::Int(1), Value::Null];
        assert_eq!(decode_history(&json!(["1", null]), &schema()).unwrap(), row);
        assert_eq!(
            decode_history(&json!({"name": null, "id": "1"}), &schema()).unwrap(),
            row
        );
    }

    #[test]
    fn history_errors() {
        assert!(matches!(
            decode_history(&json!(["1"]), &schema()),
            Err(DecodeError::Length {
                got: 1,
                expected: 2
            })
        ));
        assert!(matches!(
            decode_history(&json!({"id": "1"}), &schema()),
            Err(DecodeError::MissingColumn(c)) if c == "name"
        ));
        assert!(matches!(
            decode_history(&json!(["x", null]), &schema()),
            Err(DecodeError::Value { column, .. }) if column == "id"
        ));
    }
}
