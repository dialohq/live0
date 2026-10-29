use jiff::civil;
use jiff::tz::{AmbiguousOffset, TimeZone};
use serde_json::Value as Json;

use crate::data_types::HistoryError;
use crate::data_types::sink::{SinkNumber, java_double};

const NANOS: i128 = 1_000_000_000;

/// `DateTime` is `precision: 0, wide: false`; values are ticks of `10^-precision` seconds since the epoch.
#[derive(Debug, Clone, PartialEq)]
pub struct DateTimeType {
    pub(crate) precision: u8,
    pub(crate) wide: bool,
    pub(crate) timezone: Option<String>,
}

impl DateTimeType {
    /// As ClickHouse reads the Sink's JSON with `date_time_input_format = 'best_effort'`.
    pub(crate) fn decode_event(&self, json: Option<&Json>) -> Option<i64> {
        let nanos = match json {
            None | Some(Json::Null) => 0,
            Some(Json::Number(n)) => match SinkNumber::new(n)? {
                SinkNumber::Int(i) => i128::from(i) * NANOS,
                SinkNumber::Float(x) => decimal_nanos(&java_double(x))?,
            },
            Some(Json::String(s)) => self.parse_text(s)?,
            Some(_) => return None,
        };
        self.ticks(nanos)
    }

    /// `DateTime` saturates to its 32-bit range; `DateTime64` only fails when the ticks overflow.
    fn ticks(&self, nanos: i128) -> Option<i64> {
        if !self.wide {
            return Some((nanos / NANOS).clamp(0, i128::from(u32::MAX)) as i64);
        }
        i64::try_from(nanos / 10i128.pow(9 - u32::from(self.precision))).ok()
    }

    fn parse_text(&self, s: &str) -> Option<i128> {
        if all_digits(s) {
            return self.parse_digits(s);
        }
        // A timestamp with spaces around it is rejected, unlike a date.
        if all_digits(s.trim_matches(' ')) {
            return None;
        }
        if self.wide
            && let Some((seconds, fraction)) = s.split_once('.')
            && all_digits(seconds)
            && all_digits(fraction)
        {
            return decimal_nanos(s);
        }
        let parsed = Parser::new(s.trim_matches(' ')).parse()?;
        let (year, month, day) = parsed.date;
        let local = self
            .date(year, month, day)?
            .checked_add(jiff::Span::new().seconds(parsed.seconds))
            .ok()?;
        let offset = match parsed.offset {
            Some(offset) => offset,
            None => self.offset(local)?,
        };
        Some((i128::from(epoch_seconds(local)) - i128::from(offset)) * NANOS + parsed.nanos)
    }

    /// A unix timestamp in seconds, milliseconds, microseconds or nanoseconds, a compact date, or a year.
    fn parse_digits(&self, s: &str) -> Option<i128> {
        let n: i128 = s.parse().ok()?;
        match s.len() {
            1 | 2 | 4 => {
                let n = i32::try_from(n).ok()?;
                let local = match s.len() {
                    4 => self.date(n, 1, 1)?,
                    _ => self.date(0, 1, n)?,
                };
                Some((i128::from(epoch_seconds(local)) - i128::from(self.offset(local)?)) * NANOS)
            }
            9 | 10 => Some(n * NANOS),
            13 => Some(n * 1_000_000),
            16 => Some(n * 1_000),
            19 => Some(n),
            8 | 14 => {
                let part = |r: std::ops::Range<usize>| s.get(r)?.parse::<i32>().ok();
                let date = civil_date(part(0..4)?, part(4..6)?, part(6..8)?)?;
                let seconds = match s.len() {
                    14 => part(8..10)? * 3600 + part(10..12)? * 60 + part(12..14)?,
                    _ => 0,
                };
                let local = date.checked_add(jiff::Span::new().seconds(seconds)).ok()?;
                Some((i128::from(epoch_seconds(local)) - i128::from(self.offset(local)?)) * NANOS)
            }
            _ => None,
        }
    }

    /// Year 0 is the current year in the column's time zone, and month or day 0 is the first.
    fn date(&self, year: i32, month: i32, day: i32) -> Option<civil::DateTime> {
        let year = match year {
            0 => i32::from(jiff::Timestamp::now().to_zoned(self.tz()?).year()),
            year => year,
        };
        civil_date(year, month.max(1), day.max(1))
    }

    fn tz(&self) -> Option<TimeZone> {
        match &self.timezone {
            // The server's time zone, which live0 assumes is UTC.
            None => Some(TimeZone::UTC),
            Some(name) => TimeZone::get(name).ok(),
        }
    }

    /// The column's UTC offset at a local time: after the change in a gap, before it in an overlap.
    fn offset(&self, local: civil::DateTime) -> Option<i32> {
        let offset = match self.tz()?.to_ambiguous_timestamp(local).offset() {
            AmbiguousOffset::Unambiguous { offset } => offset,
            AmbiguousOffset::Gap { after, .. } => after,
            AmbiguousOffset::Fold { before, .. } => before,
        };
        Some(offset.seconds())
    }

    /// As written with `date_time_output_format = 'unix_timestamp'`: seconds with exactly `precision` decimals.
    pub(crate) fn decode_history(&self, json: &Json) -> Result<i64, HistoryError> {
        json.as_str()
            .and_then(|s| self.parse_unix(s))
            .ok_or_else(|| HistoryError::new(self.name(), json))
    }

    fn parse_unix(&self, s: &str) -> Option<i64> {
        let (negative, s) = match s.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, s),
        };
        let (seconds, fraction) = match s.split_once('.') {
            Some((seconds, fraction)) => (seconds, fraction),
            None => (s, ""),
        };
        if fraction.len() != usize::from(self.precision) || !all_digits(seconds) {
            return None;
        }
        // ClickHouse writes a negative fraction's digits as `'0' + digit` when the seconds are 0.
        if !negative && seconds == "0" && fraction.bytes().any(|b| (b'\''..b'0').contains(&b)) {
            return fraction.bytes().try_fold(0i64, |ticks, b| {
                (b'\''..=b'0')
                    .contains(&b)
                    .then(|| ticks * 10 + i64::from(b) - i64::from(b'0'))
            });
        }
        if !fraction.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let ticks = seconds
            .parse::<i64>()
            .ok()?
            .checked_mul(10i64.pow(u32::from(self.precision)))?
            .checked_add(if fraction.is_empty() {
                0
            } else {
                fraction.parse().ok()?
            })?;
        Some(if negative { -ticks } else { ticks })
    }

    pub(crate) fn name(&self) -> String {
        let tz = self.timezone.as_ref().map(|tz| format!("'{tz}'"));
        match (self.wide, tz) {
            (false, None) => "DateTime".to_owned(),
            (false, Some(tz)) => format!("DateTime({tz})"),
            (true, None) => format!("DateTime64({})", self.precision),
            (true, Some(tz)) => format!("DateTime64({}, {tz})", self.precision),
        }
    }

    /// `DateTime`, `DateTime('tz')`, `DateTime64(p)` or `DateTime64(p, 'tz')`.
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        let (wide, args) = if let Some(rest) = raw.strip_prefix("DateTime64") {
            (true, rest)
        } else {
            (false, raw.strip_prefix("DateTime")?)
        };
        let args: Vec<&str> = match args {
            "" => Vec::new(),
            _ => args
                .strip_prefix('(')?
                .strip_suffix(')')?
                .split(',')
                .map(str::trim)
                .collect(),
        };
        let quoted = |s: &str| {
            s.strip_prefix('\'')?
                .strip_suffix('\'')
                .filter(|tz| !tz.is_empty())
                .map(str::to_owned)
        };
        let (precision, timezone) = match (wide, args.as_slice()) {
            (false, []) => (0, None),
            (false, [tz]) => (0, Some(quoted(tz)?)),
            (true, [p]) => (p.parse().ok()?, None),
            (true, [p, tz]) => (p.parse().ok()?, Some(quoted(tz)?)),
            _ => return None,
        };
        if let Some(tz) = &timezone {
            TimeZone::get(tz).ok()?;
        }
        (precision <= 9).then_some(Self {
            precision,
            wide,
            timezone,
        })
    }
}

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// A decimal like the Sink writes numbers, `1.73E9` or `-1.5`, as nanoseconds truncated toward zero.
fn decimal_nanos(s: &str) -> Option<i128> {
    let (negative, s) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let (mantissa, exponent) = match s.split_once(['e', 'E']) {
        Some((m, e)) => (m, e.parse::<i32>().ok()?),
        None => (s, 0),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if whole.is_empty()
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let digits = format!("{whole}{fraction}")
        .trim_start_matches('0')
        .to_owned();
    let shift = exponent - fraction.len() as i32 + 9;
    let nanos = if digits.is_empty() {
        0
    } else if shift >= 0 {
        // More than 38 digits can't fit, and an i64 of ticks needs far fewer anyway.
        if digits.len() as i32 + shift > 38 {
            return None;
        }
        digits.parse::<i128>().ok()? * 10i128.pow(shift as u32)
    } else {
        let keep = digits.len() as i32 + shift;
        match keep {
            ..=0 => 0,
            _ => digits.get(..keep as usize)?.parse::<i128>().ok()?,
        }
    };
    Some(if negative { -nanos } else { nanos })
}

fn civil_date(year: i32, month: i32, day: i32) -> Option<civil::DateTime> {
    let date = civil::Date::new(
        i16::try_from(year).ok()?,
        i8::try_from(month).ok()?,
        i8::try_from(day).ok()?,
    )
    .ok()?;
    Some(date.to_datetime(civil::Time::midnight()))
}

/// Seconds since the epoch of a local time read as UTC.
fn epoch_seconds(dt: civil::DateTime) -> i64 {
    let days = i64::from(
        dt.date()
            .since(civil::date(1970, 1, 1))
            .map_or(0, |s| s.get_days()),
    );
    days * 86400
        + i64::from(dt.hour()) * 3600
        + i64::from(dt.minute()) * 60
        + i64::from(dt.second())
}

/// A date and time as `best_effort` reads it; the time of day may overflow into the next days.
struct Parsed {
    /// Year, month and day, any of which may be 0.
    date: (i32, i32, i32),
    seconds: i64,
    nanos: i128,
    offset: Option<i32>,
}

struct Parser<'a> {
    rest: &'a str,
}

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
const WEEKDAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        Self { rest: s }
    }

    fn parse(mut self) -> Option<Parsed> {
        let date = self.date()?;
        let after = self.rest.trim_start_matches(' ');
        let (t, after) = match after.strip_prefix('T') {
            Some(rest) => (true, rest),
            None => (false, after),
        };
        let (seconds, nanos) = if after.starts_with(|c: char| c.is_ascii_digit()) {
            self.rest = after;
            self.time()?
        } else {
            if t {
                self.rest = after;
            }
            (0, 0)
        };
        let offset = self.zone()?;
        self.rest.trim_matches(' ').is_empty().then_some(Parsed {
            date,
            seconds,
            nanos,
            offset,
        })
    }

    fn date(&mut self) -> Option<(i32, i32, i32)> {
        if let Some(word) = self.word()
            && WEEKDAYS.contains(&word.to_ascii_lowercase().get(..3)?)
        {
            self.rest = self
                .rest
                .get(word.len()..)?
                .trim_start_matches(',')
                .trim_start();
        }
        if let Some(month) = self.month_name() {
            self.skip(' ')?;
            let day = self.number(1, 2)?;
            self.rest = self.rest.trim_start_matches(',');
            self.skip(' ')?;
            let year = self.number(4, 4)?;
            return Some((year, month, day));
        }
        let first = self.digits();
        match first.len() {
            4 => {
                let year = self.number(4, 4)?;
                let sep = self.separator(&['-', '/'])?;
                let month = self.number(1, 2)?;
                let day = match self.rest.strip_prefix(sep) {
                    Some(rest) => {
                        self.rest = rest;
                        self.number(1, 2)?
                    }
                    None => 1,
                };
                Some((year, month, day))
            }
            1 | 2 => {
                let day = self.number(1, 2)?;
                if self.rest.starts_with(' ') {
                    self.skip(' ')?;
                    let month = self.month_name()?;
                    self.skip(' ')?;
                    let year = self.number(4, 4)?;
                    return Some((year, month, day));
                }
                let sep = self.separator(&['/', '.', '-'])?;
                let month = self.number(1, 2)?;
                self.skip(sep)?;
                let year = match self.digits().len() {
                    2 => match self.number(2, 2)? {
                        yy @ ..70 => 2000 + yy,
                        yy => 1900 + yy,
                    },
                    _ => self.number(4, 4)?,
                };
                // Day first, unless the second number can only be a day.
                match month > 12 {
                    true => Some((year, day, month)),
                    false => Some((year, month, day)),
                }
            }
            _ => None,
        }
    }

    fn time(&mut self) -> Option<(i64, i128)> {
        let hour = self.number(1, 2)?;
        self.skip(':')?;
        let minute = self.number(1, 2)?;
        let second = match self.rest.strip_prefix(':') {
            Some(rest) => {
                self.rest = rest;
                self.number(1, 2)?
            }
            None => 0,
        };
        let mut nanos = 0;
        if let Some(rest) = self.rest.strip_prefix('.') {
            self.rest = rest;
            let digits = self.digits();
            if digits.is_empty() {
                return None;
            }
            let kept = &digits[..digits.len().min(9)];
            nanos = kept.parse::<i128>().ok()? * 10i128.pow(9 - kept.len() as u32);
            self.rest = &self.rest[digits.len()..];
        }
        let rest = self.rest.trim_start_matches(' ');
        let hour = match rest.get(..2).map(str::to_ascii_lowercase).as_deref() {
            Some(half @ ("am" | "pm")) => {
                self.rest = &rest[2..];
                match (half, hour) {
                    ("am", 12) => 0,
                    ("pm", h) if h < 12 => h + 12,
                    (_, h) => h,
                }
            }
            _ => hour,
        };
        Some((i64::from(hour * 3600 + minute * 60 + second), nanos))
    }

    /// `Z`, `UTC`, `GMT`, `±H[H]`, `±HMM`, `±HHMM` or `±H[H]:MM`, optionally after spaces.
    fn zone(&mut self) -> Option<Option<i32>> {
        let rest = self.rest.trim_start_matches(' ');
        for name in ["Z", "UTC", "GMT"] {
            if let Some(after) = rest.strip_prefix(name) {
                self.rest = after;
                return Some(Some(0));
            }
        }
        let sign = match rest.chars().next() {
            Some('+') => 1,
            Some('-') => -1,
            _ => return Some(None),
        };
        self.rest = &rest[1..];
        let digits = self.digits();
        let (hours, minutes) = match (digits.len(), self.rest[digits.len()..].starts_with(':')) {
            (1 | 2, true) => {
                let hours = self.number(1, 2)?;
                self.skip(':')?;
                (hours, self.two_digits()?)
            }
            (1 | 2, false) => (self.number(1, 2)?, 0),
            (3, _) => {
                let n = self.number(3, 3)?;
                (n / 100, n % 100)
            }
            (4, _) => {
                let n = self.number(4, 4)?;
                (n / 100, n % 100)
            }
            _ => return None,
        };
        Some(Some(sign * (hours * 3600 + minutes * 60)))
    }

    fn month_name(&mut self) -> Option<i32> {
        let word = self.word()?;
        let lower = word.to_ascii_lowercase();
        let month = MONTHS.iter().position(|m| lower.starts_with(m))?;
        self.rest = &self.rest[word.len()..];
        i32::try_from(month + 1).ok()
    }

    fn word(&self) -> Option<&'a str> {
        let end = self
            .rest
            .find(|c: char| !c.is_ascii_alphabetic())
            .unwrap_or(self.rest.len());
        (end > 0).then(|| &self.rest[..end])
    }

    fn digits(&self) -> &'a str {
        let end = self
            .rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(self.rest.len());
        &self.rest[..end]
    }

    fn number(&mut self, min: usize, max: usize) -> Option<i32> {
        let digits = self.digits();
        if !(min..=max).contains(&digits.len()) {
            return None;
        }
        self.rest = &self.rest[digits.len()..];
        digits.parse().ok()
    }

    fn two_digits(&mut self) -> Option<i32> {
        let n = self.rest.get(..2).filter(|d| all_digits(d))?.parse().ok()?;
        self.rest = &self.rest[2..];
        Some(n)
    }

    fn separator(&mut self, allowed: &[char]) -> Option<char> {
        let c = self.rest.chars().next().filter(|c| allowed.contains(c))?;
        self.rest = &self.rest[1..];
        Some(c)
    }

    fn skip(&mut self, c: char) -> Option<()> {
        self.rest = self.rest.strip_prefix(c)?;
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_types() {
        for raw in [
            "DateTime",
            "DateTime('UTC')",
            "DateTime64(0)",
            "DateTime64(3)",
            "DateTime64(6, 'Europe/Warsaw')",
        ] {
            assert_eq!(
                DateTimeType::parse(raw).map(|t| t.name()).as_deref(),
                Some(raw)
            );
        }
        for raw in [
            "DateTime64",
            "DateTime64(10)",
            "DateTime()",
            "DateTime('')",
            "DateTime32",
        ] {
            assert_eq!(DateTimeType::parse(raw), None, "{raw}");
        }
    }

    #[test]
    fn history_unix_timestamps() {
        let ms = DateTimeType::parse("DateTime64(3)").unwrap();
        assert_eq!(
            ms.decode_history(&Json::from("1730000000.123")).unwrap(),
            1730000000123
        );
        assert_eq!(ms.decode_history(&Json::from("-1.500")).unwrap(), -1500);
        assert!(ms.decode_history(&Json::from("1.5")).is_err());
        assert_eq!(ms.decode_history(&Json::from("0.+00")).unwrap(), -500);
        assert_eq!(ms.decode_history(&Json::from("0..+0")).unwrap(), -250);
        assert_eq!(ms.decode_history(&Json::from("0.00+")).unwrap(), -5);
        let s = DateTimeType::parse("DateTime").unwrap();
        assert_eq!(
            s.decode_history(&Json::from("4294967295")).unwrap(),
            4294967295
        );
    }

    /// Not decoder cases, since what the Sink stored depends on the year they were recorded in.
    #[test]
    fn missing_year_is_this_year() {
        let utc = DateTimeType::parse("DateTime").unwrap();
        let year = jiff::Timestamp::now().to_zoned(TimeZone::UTC).year();
        let start = |month, day| {
            let local = civil_date(i32::from(year), month, day).unwrap();
            Some(epoch_seconds(local))
        };
        assert_eq!(
            utc.decode_event(Some(&Json::from("0000-00-00 00:00:00"))),
            start(1, 1)
        );
        assert_eq!(
            utc.decode_event(Some(&Json::from("0000-10-27"))),
            start(10, 27)
        );
        assert_eq!(utc.decode_event(Some(&Json::from("1"))), start(1, 1));
        assert_eq!(utc.decode_event(Some(&Json::from("27"))), start(1, 27));
    }

    #[test]
    fn decoder_cases() {
        crate::data_types::cases::check(include_str!("../../tests/results/datetime.jsonl"));
    }
}
