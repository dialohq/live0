use std::fmt::Write;

use serde_json::{Number, Value as Json};

/// A JSON number as the Sink passes it on to ClickHouse.
pub(crate) enum SinkNumber {
    /// Written without a fraction or exponent; the Sink keeps only the low 64 bits.
    Int(i64),
    /// Anything else, read as an `f64` with negative zero turned into zero.
    Float(f64),
}

impl SinkNumber {
    /// Needs `arbitrary_precision` so that the number's original text is still there.
    pub(crate) fn new(n: &Number) -> Option<Self> {
        let text = n.as_str();
        if text.contains(['.', 'e', 'E']) {
            let x: f64 = text.parse().ok()?;
            return Some(Self::Float(if x == 0.0 { 0.0 } else { x }));
        }
        let (negative, digits) = match text.strip_prefix('-') {
            Some(digits) => (true, digits),
            None => (false, text),
        };
        let low = wrapping_digits(digits)?;
        Some(Self::Int(
            if negative { low.wrapping_neg() } else { low } as i64
        ))
    }
}

/// The digits as a number modulo 2^64.
pub(crate) fn wrapping_digits(digits: &str) -> Option<u64> {
    digits.bytes().try_fold(0u64, |n, b| {
        b.is_ascii_digit()
            .then(|| n.wrapping_mul(10).wrapping_add(u64::from(b - b'0')))
    })
}

/// The text the Sink sends for a non-string value.
pub(crate) fn text(json: &Json) -> String {
    let mut out = String::new();
    write_text(&mut out, json);
    out
}

fn write_text(out: &mut String, json: &Json) {
    match json {
        Json::Number(n) => match SinkNumber::new(n) {
            Some(SinkNumber::Int(i)) => {
                let _ = write!(out, "{i}");
            }
            Some(SinkNumber::Float(x)) => out.push_str(&java_double(x)),
            None => out.push_str(n.as_str()),
        },
        Json::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_text(out, item);
            }
            out.push(']');
        }
        Json::Object(map) => {
            out.push('{');
            for (i, (key, value)) in map.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Json::String(key.clone()).to_string());
                out.push(':');
                write_text(out, value);
            }
            out.push('}');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// Java's `Double.toString`, which is how the Sink writes non-integer numbers.
pub(crate) fn java_double(x: f64) -> String {
    if x.is_infinite() {
        return if x > 0.0 { "Infinity" } else { "-Infinity" }.to_owned();
    }
    if x == 0.0 {
        return "0.0".to_owned();
    }
    if (1e-3..1e7).contains(&x.abs()) {
        let s = x.to_string();
        return if s.contains('.') { s } else { s + ".0" };
    }
    let s = format!("{x:e}");
    let (mantissa, exponent) = s.split_once('e').unwrap_or((&s, "0"));
    if mantissa.contains('.') {
        format!("{mantissa}E{exponent}")
    } else {
        format!("{mantissa}.0E{exponent}")
    }
}
