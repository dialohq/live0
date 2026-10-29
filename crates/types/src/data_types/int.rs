use serde_json::Value as Json;

use crate::data_types::HistoryError;
use crate::data_types::sink::{SinkNumber, wrapping_digits};

#[derive(Debug, Clone, PartialEq)]
pub struct IntType {
    pub(crate) signed: bool,
    pub(crate) width: u8,
}

impl IntType {
    pub(crate) fn decode_event(&self, json: Option<&Json>) -> Option<i128> {
        let n = match json {
            None | Some(Json::Null) => 0,
            Some(Json::Bool(b)) => i128::from(*b),
            Some(Json::Number(n)) => match SinkNumber::new(n)? {
                SinkNumber::Int(i) => i128::from(i),
                SinkNumber::Float(_) => return None,
            },
            Some(Json::String(s)) => self.parse(s)?,
            Some(_) => return None,
        };
        if !self.signed && n < 0 {
            return None;
        }
        Some(self.wrap(n))
    }

    pub(crate) fn decode_history(&self, json: &Json) -> Result<i128, HistoryError> {
        let n: Option<i128> = match json {
            Json::Number(n) => n.as_str().parse().ok(),
            // 64-bit integers, as `output_format_json_quote_64bit_integers = 1` writes them.
            Json::String(s) if self.width == 64 => s.parse().ok(),
            _ => None,
        };
        n.filter(|n| (self.min()..=self.max()).contains(n))
            .ok_or_else(|| HistoryError::new(self.name(), json))
    }

    fn name(&self) -> String {
        format!("{}Int{}", if self.signed { "" } else { "U" }, self.width)
    }

    /// How ClickHouse reads a quoted integer: an optional sign, then digits that wrap on overflow.
    fn parse(&self, s: &str) -> Option<i128> {
        if s.is_empty() {
            return Some(0);
        }
        let (negative, digits) = match s.as_bytes()[0] {
            b'-' if self.signed => (true, &s[1..]),
            b'-' => return None,
            b'+' => (false, &s[1..]),
            _ => (false, s),
        };
        if digits.is_empty() {
            return None;
        }
        let n = i128::from(wrapping_digits(digits)?);
        Some(if negative { -n } else { n })
    }

    fn wrap(&self, n: i128) -> i128 {
        let m = n.rem_euclid(1 << self.width);
        match self.signed && m > self.max() {
            true => m - (1 << self.width),
            false => m,
        }
    }

    fn min(&self) -> i128 {
        match self.signed {
            true => -(1 << (self.width - 1)),
            false => 0,
        }
    }

    fn max(&self) -> i128 {
        match self.signed {
            true => (1 << (self.width - 1)) - 1,
            false => (1 << self.width) - 1,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn decoder_cases() {
        crate::data_types::cases::check(include_str!("../../tests/results/int.jsonl"));
    }
}
