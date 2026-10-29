use ethnum::{I256, U256};
use serde_json::Value as Json;

use crate::Value;
use crate::data_types::HistoryError;
use crate::data_types::sink::{SinkNumber, wrapping_digits};

#[derive(Debug, Clone, PartialEq)]
pub struct IntType {
    pub(crate) signed: bool,
    pub(crate) width: u16,
}

impl IntType {
    pub fn is_signed(&self) -> bool {
        self.signed
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    /// Up to 64 bits as [`Value::Int`]; wider as [`Value::BigInt`], or [`Value::BigUInt`] for `UInt256`.
    pub(crate) fn decode_event(&self, json: Option<&Json>) -> Option<Value> {
        if self.width <= 64 {
            return self.narrow_event(json).map(Value::Int);
        }
        let raw = match json {
            None | Some(Json::Null) => U256::ZERO,
            Some(Json::Bool(b)) => U256::from(u8::from(*b)),
            // The Sink only passes on a number's low 64 bits.
            Some(Json::Number(n)) => match SinkNumber::new(n)? {
                SinkNumber::Int(i) if !self.signed && i < 0 => return None,
                SinkNumber::Int(i) => I256::from(i).as_u256(),
                SinkNumber::Float(_) => return None,
            },
            Some(Json::String(s)) => self.parse_wide(s)?,
            Some(_) => return None,
        };
        Some(self.wide_value(raw))
    }

    pub(crate) fn decode_history(&self, json: &Json) -> Result<Value, HistoryError> {
        if self.width <= 64 {
            return self.narrow_history(json).map(Value::Int);
        }
        let value = json.as_str().and_then(|s| match (self.signed, self.width) {
            (false, 256) => U256::from_str_radix(s, 10).ok().map(Value::BigUInt),
            (true, 256) => I256::from_str_radix(s, 10).ok().map(Value::BigInt),
            (signed, _) => {
                let n = I256::from_str_radix(s, 10).ok()?;
                let (min, max) = match signed {
                    true => (-(I256::ONE << 127u32), (I256::ONE << 127u32) - 1),
                    false => (I256::ZERO, (I256::ONE << 128u32) - 1),
                };
                (min..=max).contains(&n).then_some(Value::BigInt(n))
            }
        });
        value.ok_or_else(|| HistoryError::new(self.name(), json))
    }

    /// Like [`Self::parse`], but wrapping at the type's full width.
    fn parse_wide(&self, s: &str) -> Option<U256> {
        if s.is_empty() {
            return Some(U256::ZERO);
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
        let n = digits.bytes().try_fold(U256::ZERO, |n, b| {
            b.is_ascii_digit().then(|| {
                n.wrapping_mul(U256::new(10))
                    .wrapping_add(U256::from(b - b'0'))
            })
        })?;
        Some(if negative { n.wrapping_neg() } else { n })
    }

    /// The low `width` bits of a two's complement number, as the type reads them.
    fn wide_value(&self, raw: U256) -> Value {
        let bits = u32::from(self.width);
        let raw = match bits {
            256 => raw,
            _ => raw & ((U256::ONE << bits) - 1),
        };
        match (self.signed, bits) {
            (false, 256) => Value::BigUInt(raw),
            (true, 256) | (false, _) => Value::BigInt(raw.as_i256()),
            (true, _) if raw >> (bits - 1) != U256::ZERO => {
                Value::BigInt(raw.as_i256() - (I256::ONE << bits))
            }
            (true, _) => Value::BigInt(raw.as_i256()),
        }
    }

    fn narrow_event(&self, json: Option<&Json>) -> Option<i128> {
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

    fn narrow_history(&self, json: &Json) -> Result<i128, HistoryError> {
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
