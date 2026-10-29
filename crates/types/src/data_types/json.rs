use serde_json::{Map, Number, Value as Json};

use crate::data_types::HistoryError;
use crate::data_types::sink::SinkNumber;

#[derive(Debug, Clone, PartialEq)]
pub struct JsonType;

impl JsonType {
    pub(crate) fn decode_event(&self, json: Option<&Json>) -> Option<Map<String, Json>> {
        match json {
            None | Some(Json::Null) => Some(Map::new()),
            Some(Json::Object(map)) => Some(event_object(map)),
            Some(_) => None,
        }
    }

    pub(crate) fn decode_history(&self, json: &Json) -> Result<Map<String, Json>, HistoryError> {
        match history_value(json) {
            Json::Object(map) => Ok(map),
            _ => Err(HistoryError::new("JSON", json)),
        }
    }
}

/// What ClickHouse keeps of an object: dotted keys become paths, while nulls and empty objects are dropped.
fn event_object(map: &Map<String, Json>) -> Map<String, Json> {
    let mut out = Map::new();
    for (key, value) in map {
        let value = match value {
            Json::Null => continue,
            Json::Object(inner) => {
                let inner = event_object(inner);
                if inner.is_empty() {
                    continue;
                }
                Json::Object(inner)
            }
            other => event_value(other),
        };
        insert_path(&mut out, key, value);
    }
    out
}

fn event_value(json: &Json) -> Json {
    match json {
        Json::Number(n) => match SinkNumber::new(n) {
            Some(SinkNumber::Int(i)) => Json::String(i.to_string()),
            Some(SinkNumber::Float(x)) => float(x),
            None => json.clone(),
        },
        Json::Array(items) => Json::Array(items.iter().map(event_value).collect()),
        Json::Object(map) => Json::Object(event_object(map)),
        other => other.clone(),
    }
}

/// Integers in history are already quoted, so only floats need a canonical form.
fn history_value(json: &Json) -> Json {
    match json {
        Json::Number(n) => n.as_str().parse().map_or_else(|_| json.clone(), float),
        Json::Array(items) => Json::Array(items.iter().map(history_value).collect()),
        Json::Object(map) => Json::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), history_value(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Floats in the same form on both sides, with infinities as `output_format_json_quote_denormals` writes them.
fn float(x: f64) -> Json {
    match Number::from_f64(x) {
        Some(n) => Json::Number(n),
        None if x > 0.0 => Json::String("inf".to_owned()),
        None if x < 0.0 => Json::String("-inf".to_owned()),
        None => Json::String("nan".to_owned()),
    }
}

fn insert_path(map: &mut Map<String, Json>, key: &str, value: Json) {
    let Some((head, rest)) = key.split_once('.') else {
        match (map.get_mut(key), value) {
            (Some(Json::Object(existing)), Json::Object(new)) => {
                for (k, v) in new {
                    insert_path(existing, &k, v);
                }
            }
            (_, value) => {
                map.insert(key.to_owned(), value);
            }
        }
        return;
    };
    let entry = map
        .entry(head.to_owned())
        .or_insert_with(|| Json::Object(Map::new()));
    if !entry.is_object() {
        *entry = Json::Object(Map::new());
    }
    if let Json::Object(inner) = entry {
        insert_path(inner, rest, value);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn decoder_cases() {
        crate::data_types::cases::check(include_str!("../../tests/results/json.jsonl"));
    }
}
