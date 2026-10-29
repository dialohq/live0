use serde_json::Value as Json;

use crate::data_types::HistoryError;
use crate::data_types::sink::{SinkNumber, java_double};

#[derive(Debug, Clone, PartialEq)]
pub struct FloatType {
    pub(crate) width: u8,
}

impl FloatType {
    pub(crate) fn decode_event(&self, json: Option<&Json>) -> Option<f64> {
        match json {
            None | Some(Json::Null) => Some(0.0),
            Some(Json::Bool(b)) => Some(f64::from(u8::from(*b))),
            Some(Json::Number(n)) => match SinkNumber::new(n)? {
                SinkNumber::Int(i) => Some(match self.width {
                    32 => f64::from(i as f32),
                    _ => i as f64,
                }),
                // ClickHouse reads the Sink's text, which may round differently than `x as f32`.
                SinkNumber::Float(x) => self.parse(&java_double(x)),
            },
            Some(Json::String(s)) => self.parse(s),
            Some(_) => None,
        }
    }

    pub(crate) fn decode_history(&self, json: &Json) -> Result<f64, HistoryError> {
        let x = match json {
            Json::Number(n) => self.parse(n.as_str()),
            // Only written like this with `output_format_json_quote_denormals = 1`, otherwise as null.
            Json::String(s) => match s.as_str() {
                "nan" => Some(f64::NAN),
                "inf" => Some(f64::INFINITY),
                "-inf" => Some(f64::NEG_INFINITY),
                _ => None,
            },
            _ => None,
        };
        x.ok_or_else(|| HistoryError::new(format!("Float{}", self.width), json))
    }

    fn parse(&self, s: &str) -> Option<f64> {
        match self.width {
            32 => s.parse::<f32>().ok().map(f64::from),
            _ => s.parse().ok(),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn decoder_cases() {
        crate::data_types::cases::check(include_str!("../../tests/results/float.jsonl"));
    }
}
