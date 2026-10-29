use std::borrow::Cow;
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

/// Drops the leading zeros of numbers outside strings, which the Sink accepts and serde_json doesn't.
pub(crate) fn strip_leading_zeros(raw: &str) -> Cow<'_, str> {
    let bytes = raw.as_bytes();
    let mut out = String::new();
    let mut copied = 0;
    let (mut in_string, mut escaped) = (false, false);
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            match b {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
        } else if b == b'"' {
            in_string = true;
        } else if b == b'0' && starts_number(bytes, i) {
            let mut end = i;
            while bytes.get(end) == Some(&b'0')
                && bytes.get(end + 1).is_some_and(u8::is_ascii_digit)
            {
                end += 1;
            }
            if end > i {
                out.push_str(&raw[copied..i]);
                copied = end;
                i = end;
                continue;
            }
        }
        i += 1;
    }
    match copied {
        0 => Cow::Borrowed(raw),
        _ => {
            out.push_str(&raw[copied..]);
            Cow::Owned(out)
        }
    }
}

/// Whether the byte at `i` is a number's first digit rather than one inside its fraction or exponent.
fn starts_number(bytes: &[u8], i: usize) -> bool {
    match i.checked_sub(1).map(|p| bytes[p]) {
        None => true,
        Some(b'-') => !matches!(i.checked_sub(2).map(|p| bytes[p]), Some(b'e' | b'E')),
        Some(prev) => !(prev.is_ascii_alphanumeric() || matches!(prev, b'.' | b'+')),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_leading_zeros() {
        for (raw, stripped) in [
            ("01", "1"),
            ("007", "7"),
            ("00", "0"),
            ("-01.5", "-1.5"),
            ("-00", "-0"),
            ("00.5", "0.5"),
            ("001e2", "1e2"),
            (
                "{\"a\": [01, {\"b\": -007.5}]}",
                "{\"a\": [1, {\"b\": -7.5}]}",
            ),
        ] {
            assert_eq!(strip_leading_zeros(raw), stripped, "{raw}");
        }
    }

    #[test]
    fn keeps_everything_else() {
        for raw in [
            "0",
            "-0",
            "0.5",
            "10",
            "100.007",
            "1e-05",
            "1E+005",
            "\"01\"",
            "{\"01\": \"a\\\"01\"}",
            "[0, 0.0, -0.0]",
        ] {
            assert!(
                matches!(strip_leading_zeros(raw), Cow::Borrowed(_)),
                "{raw}"
            );
        }
    }
}
