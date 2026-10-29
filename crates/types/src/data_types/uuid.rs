use serde_json::Value as Json;

use crate::data_types::HistoryError;

#[derive(Debug, Clone, PartialEq)]
pub struct UuidType;

impl UuidType {
    /// 32 hex digits in either case, with hyphens at the canonical 8-4-4-4-12 positions or none.
    pub(crate) fn decode_event(&self, json: Option<&Json>) -> Option<u128> {
        match json {
            None | Some(Json::Null) => Some(0),
            Some(Json::String(s)) => parse(s),
            Some(_) => None,
        }
    }

    pub(crate) fn decode_history(&self, json: &Json) -> Result<u128, HistoryError> {
        json.as_str()
            .and_then(parse)
            .ok_or_else(|| HistoryError::new("UUID", json))
    }
}

fn parse(s: &str) -> Option<u128> {
    let hex: String = match s.len() {
        32 => s.to_owned(),
        36 => {
            let b = s.as_bytes();
            if [8, 13, 18, 23].iter().any(|&i| b[i] != b'-') {
                return None;
            }
            s.chars().filter(|&c| c != '-').collect()
        }
        _ => return None,
    };
    if hex.len() != 32 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u128::from_str_radix(&hex, 16).ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn decoder_cases() {
        crate::data_types::cases::check(include_str!("../../tests/results/uuid.jsonl"));
    }
}
