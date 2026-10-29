use ethnum::I256;
use serde_json::Value as Json;

use crate::data_types::HistoryError;
use crate::data_types::sink::{SinkNumber, java_double};

/// Values are the number times `10^scale`, below `10^precision` in magnitude.
#[derive(Debug, Clone, PartialEq)]
pub struct DecimalType {
    pub(crate) precision: u8,
    pub(crate) scale: u8,
}

impl DecimalType {
    /// As ClickHouse reads the Sink's JSON: extra fraction digits are cut, too many integer digits reject.
    pub(crate) fn decode_event(&self, json: Option<&Json>) -> Option<I256> {
        match json {
            None | Some(Json::Null) => Some(I256::ZERO),
            Some(Json::Number(n)) => match SinkNumber::new(n)? {
                SinkNumber::Int(i) => self.fit(I256::from(i).checked_mul(pow10(self.scale)?)?),
                SinkNumber::Float(x) => self.parse(&java_double(x)),
            },
            Some(Json::String(s)) => self.parse(s),
            Some(_) => None,
        }
    }

    /// As written with `output_format_json_quote_decimals = 1`: exact, without trailing zeros.
    pub(crate) fn decode_history(&self, json: &Json) -> Result<I256, HistoryError> {
        json.as_str()
            .and_then(|s| {
                let (_, fraction) = s.split_once('.').unwrap_or((s, ""));
                (fraction.len() <= usize::from(self.scale)).then(|| self.parse(s))?
            })
            .ok_or_else(|| HistoryError::new(self.name(), json))
    }

    pub(crate) fn name(&self) -> String {
        format!("Decimal({}, {})", self.precision, self.scale)
    }

    /// `Decimal(P, S)`, or `Decimal32(S)` to `Decimal256(S)` with their fixed precisions.
    pub(crate) fn parse_type(raw: &str) -> Option<Self> {
        let rest = raw.strip_prefix("Decimal")?;
        let (fixed, args) = match rest.split_once('(') {
            Some(("", args)) => (None, args),
            Some(("32", args)) => (Some(9), args),
            Some(("64", args)) => (Some(18), args),
            Some(("128", args)) => (Some(38), args),
            Some(("256", args)) => (Some(76), args),
            _ => return None,
        };
        let args: Vec<u8> = args
            .strip_suffix(')')?
            .split(',')
            .map(|a| a.trim().parse().ok())
            .collect::<Option<_>>()?;
        let (precision, scale) = match (fixed, args.as_slice()) {
            (None, [p, s]) => (*p, *s),
            (Some(p), [s]) => (p, *s),
            _ => return None,
        };
        ((1..=76).contains(&precision) && scale <= precision).then_some(Self { precision, scale })
    }

    /// As ClickHouse reads decimal text: an optional sign, digits where every `.` after the first is skipped,
    /// and an optional exponent whose digits may be missing; no digits at all is zero.
    fn parse(&self, s: &str) -> Option<I256> {
        let (negative, s) = match s.as_bytes().first() {
            Some(b'-') => (true, &s[1..]),
            Some(b'+') => (false, &s[1..]),
            _ => (false, s),
        };
        let (mantissa, exponent) = match s.split_once(['e', 'E']) {
            Some((m, e)) => (m, exponent(e)?),
            None => (s, 0),
        };
        let mut digits = String::new();
        let mut fraction_len: i32 = 0;
        let mut after_point = false;
        for b in mantissa.bytes() {
            match b {
                b'.' => after_point = true,
                b'0'..=b'9' => {
                    digits.push(char::from(b));
                    fraction_len += i32::from(after_point);
                }
                _ => return None,
            }
        }
        let digits = digits.trim_start_matches('0');
        let shift = exponent + i32::from(self.scale) - fraction_len;
        let value = if digits.is_empty() {
            I256::ZERO
        } else if shift >= 0 {
            if digits.len() + usize::try_from(shift).ok()? > 77 {
                return None;
            }
            I256::from_str_radix(digits, 10)
                .ok()?
                .checked_mul(pow10(u8::try_from(shift).ok()?)?)?
        } else {
            let keep = i64::try_from(digits.len()).ok()? + i64::from(shift);
            match usize::try_from(keep) {
                Ok(0) | Err(_) => I256::ZERO,
                Ok(keep) if keep > 77 => return None,
                Ok(keep) => I256::from_str_radix(&digits[..keep], 10).ok()?,
            }
        };
        self.fit(if negative { -value } else { value })
    }

    fn fit(&self, value: I256) -> Option<I256> {
        (value.unsigned_abs() < pow10(self.precision)?.unsigned_abs()).then_some(value)
    }
}

/// `[+-]digits`, where missing digits are 0.
fn exponent(s: &str) -> Option<i32> {
    let (negative, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // Far past any scale, so it saturates instead of overflowing.
    let n = digits
        .parse::<i32>()
        .unwrap_or(if digits.is_empty() { 0 } else { 100_000 });
    Some(if negative { -n } else { n })
}

fn pow10(exp: u8) -> Option<I256> {
    I256::new(10).checked_pow(u32::from(exp))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_types() {
        let t = |raw| DecimalType::parse_type(raw).map(|t| (t.precision, t.scale));
        assert_eq!(t("Decimal(9, 2)"), Some((9, 2)));
        assert_eq!(t("Decimal32(3)"), Some((9, 3)));
        assert_eq!(t("Decimal256(20)"), Some((76, 20)));
        assert_eq!(t("Decimal(77, 2)"), None);
        assert_eq!(t("Decimal(5, 6)"), None);
        assert_eq!(t("Decimal16(2)"), None);
    }

    #[test]
    fn decoder_cases() {
        crate::data_types::cases::check(include_str!("../../tests/results/decimal.jsonl"));
    }
}
