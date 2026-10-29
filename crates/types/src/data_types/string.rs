use serde_json::Value as Json;

use crate::data_types::{HistoryError, sink};

#[derive(Debug, Clone, PartialEq)]
pub struct StringType;

impl StringType {
    pub(crate) fn decode_event(&self, json: Option<&Json>) -> Option<String> {
        match json {
            None | Some(Json::Null) => Some(String::new()),
            Some(Json::String(s)) => Some(s.clone()),
            Some(other) => Some(sink::text(other)),
        }
    }

    pub(crate) fn decode_history(&self, json: &Json) -> Result<String, HistoryError> {
        json.as_str()
            .map(str::to_owned)
            .ok_or_else(|| HistoryError::new("String", json))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn decoder_cases() {
        crate::data_types::cases::check(include_str!("../../tests/results/string.jsonl"));
    }
}
