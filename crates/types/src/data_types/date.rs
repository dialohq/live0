use jiff::civil;
use serde_json::Value as Json;

use crate::data_types::HistoryError;
use crate::data_types::sink::SinkNumber;

/// `Date` is days since the epoch as a `UInt16`, `Date32` as an `Int32`.
#[derive(Debug, Clone, PartialEq)]
pub struct DateType {
    pub(crate) wide: bool,
}

impl DateType {
    /// As ClickHouse reads the Sink's JSON: whole numbers of days, or `YYYY-MM-DD` and `YYYYMMDD` text.
    pub(crate) fn decode_event(&self, json: Option<&Json>) -> Option<i32> {
        match json {
            None | Some(Json::Null) => Some(0),
            Some(Json::Bool(b)) => Some(i32::from(*b)),
            Some(Json::Number(n)) => match SinkNumber::new(n)? {
                SinkNumber::Int(i) if self.wide => Some(i as i32),
                SinkNumber::Int(i) if i < 0 => None,
                SinkNumber::Int(i) => Some(i32::from(i as u16)),
                SinkNumber::Float(_) => None,
            },
            Some(Json::String(s)) => self.parse(s),
            Some(_) => None,
        }
    }

    /// As selected by [`crate::DataType::history_expr`], since the text form clamps `Date32` to years 0–9999.
    pub(crate) fn decode_history(&self, json: &Json) -> Result<i32, HistoryError> {
        json.as_i64()
            .and_then(|days| i32::try_from(days).ok())
            .filter(|days| self.wide || (0..=i32::from(u16::MAX)).contains(days))
            .ok_or_else(|| HistoryError::new(self.name(), json))
    }

    pub(crate) fn name(&self) -> &'static str {
        if self.wide { "Date32" } else { "Date" }
    }

    pub(crate) fn history_expr(&self, column: &str) -> String {
        if self.wide {
            format!("toInt32({column})")
        } else {
            format!("toUInt16({column})")
        }
    }

    fn parse(&self, s: &str) -> Option<i32> {
        let (year, month, day) = split(s)?;
        // An impossible month or day reads as the type's first day; days 29–31 past a month's end overflow.
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return Some(if self.wide { -25567 } else { 0 });
        }
        let first = civil::Date::new(year, month, 1).ok()?;
        let days =
            i64::from(first.since(civil::date(1970, 1, 1)).ok()?.get_days()) + i64::from(day) - 1;
        if self.wide {
            i32::try_from(days).ok()
        } else {
            Some(days.clamp(0, i64::from(u16::MAX)) as i32)
        }
    }
}

/// `YYYYMMDD`, or a 4-digit year, a 1–2 digit month and day, each after one separator.
fn split(s: &str) -> Option<(i16, i8, i8)> {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if s.len() == 8 && digits(s) {
        return Some((
            s[..4].parse().ok()?,
            s[4..6].parse().ok()?,
            s[6..].parse().ok()?,
        ));
    }
    let year = s.get(..4).filter(|y| digits(y))?;
    let rest = s.get(4..)?;
    let mut parts = rest.get(1..)?.split(|c: char| !c.is_ascii_digit());
    let (month, day) = (parts.next()?, parts.next()?);
    let sep_ok = rest.starts_with(|c: char| !c.is_ascii_digit())
        && rest[1 + month.len()..].starts_with(|c: char| !c.is_ascii_digit());
    if parts.next().is_some()
        || !sep_ok
        || !(1..=2).contains(&month.len())
        || !(1..=2).contains(&day.len())
    {
        return None;
    }
    Some((year.parse().ok()?, month.parse().ok()?, day.parse().ok()?))
}

#[cfg(test)]
mod tests {
    #[test]
    fn decoder_cases() {
        crate::data_types::cases::check(include_str!("../../tests/results/date.jsonl"));
    }
}
