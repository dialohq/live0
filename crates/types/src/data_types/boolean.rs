use serde_json::Value as Json;

use crate::data_types::HistoryError;
use crate::data_types::sink::SinkNumber;

#[derive(Debug, PartialEq)]
pub struct BoolType;

impl BoolType {
    pub(crate) fn decode_event(&self, json: Option<&Json>) -> Option<bool> {
        match json {
            None | Some(Json::Null) => Some(false),
            Some(Json::Bool(b)) => Some(*b),
            Some(Json::Number(n)) => match SinkNumber::new(n)? {
                SinkNumber::Int(0) => Some(false),
                SinkNumber::Int(1) => Some(true),
                _ => None,
            },
            Some(_) => None,
        }
    }

    pub(crate) fn decode_history(&self, json: &Json) -> Result<bool, HistoryError> {
        json.as_bool()
            .ok_or_else(|| HistoryError::new("Bool", json))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn decoder_cases() {
        crate::data_types::cases::check(include_str!("../../tests/results/bool.jsonl"));
    }
}
