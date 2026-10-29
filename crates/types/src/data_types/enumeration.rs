use serde_json::Value as Json;

use crate::data_types::HistoryError;
use crate::data_types::sink::SinkNumber;

/// Values are the numbers; `wide` is `Enum16`.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumType {
    pub(crate) wide: bool,
    pub(crate) items: Vec<(String, i16)>,
}

impl EnumType {
    /// A name, a string of one of its numbers, or a number that wraps to the enum's width and is one of them.
    pub(crate) fn decode_event(&self, json: Option<&Json>) -> Option<i16> {
        match json {
            None | Some(Json::Null) => self.items.iter().map(|(_, v)| *v).min(),
            Some(Json::Number(n)) => match SinkNumber::new(n)? {
                SinkNumber::Int(i) => self.value(i),
                SinkNumber::Float(_) => None,
            },
            Some(Json::String(s)) => match self.items.iter().find(|(name, _)| name == s) {
                Some((_, v)) => Some(*v),
                None => {
                    let n = s.parse().ok()?;
                    self.items.iter().any(|(_, v)| *v == n).then_some(n)
                }
            },
            Some(_) => None,
        }
    }

    pub(crate) fn decode_history(&self, json: &Json) -> Result<i16, HistoryError> {
        json.as_str()
            .and_then(|s| self.items.iter().find(|(name, _)| name == s))
            .map(|(_, v)| *v)
            .ok_or_else(|| HistoryError::new(if self.wide { "Enum16" } else { "Enum8" }, json))
    }

    fn value(&self, n: i64) -> Option<i16> {
        let n = if self.wide {
            n as i16
        } else {
            i16::from(n as i8)
        };
        self.items.iter().any(|(_, v)| *v == n).then_some(n)
    }

    /// `Enum8('a' = 1, ...)` or `Enum16(...)`, names quoted with `'` and backslash escapes.
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        let (wide, rest) = if let Some(rest) = raw.strip_prefix("Enum16(") {
            (true, rest)
        } else {
            (false, raw.strip_prefix("Enum8(")?)
        };
        let mut chars = rest.strip_suffix(')')?.chars().peekable();
        let mut items = Vec::new();
        loop {
            while chars.next_if(|c| *c == ' ').is_some() {}
            if chars.next()? != '\'' {
                return None;
            }
            let mut name = String::new();
            loop {
                match chars.next()? {
                    '\\' => name.push(chars.next()?),
                    '\'' if chars.next_if_eq(&'\'').is_some() => name.push('\''),
                    '\'' => break,
                    c => name.push(c),
                }
            }
            let rest: String = chars.by_ref().take_while(|c| *c != ',').collect();
            let value = rest.trim().strip_prefix('=')?.trim().parse().ok()?;
            if !wide && i8::try_from(value).is_err() {
                return None;
            }
            items.push((name, value));
            if chars.peek().is_none() {
                break;
            }
        }
        Some(Self { wide, items })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_types() {
        let t =
            EnumType::parse(r"Enum8('it\'s' = 1, 'a, b' = 2, '' = -3, 'back\\slash' = 4)").unwrap();
        let names: Vec<(&str, i16)> = t.items.iter().map(|(n, v)| (n.as_str(), *v)).collect();
        assert_eq!(
            names,
            [("it's", 1), ("a, b", 2), ("", -3), ("back\\slash", 4)]
        );
        assert!(!t.wide);
        assert!(EnumType::parse("Enum16('x' = 1000)").is_some_and(|t| t.wide));
        assert_eq!(EnumType::parse("Enum8('x' = 1000)"), None);
        assert_eq!(EnumType::parse("Enum8('x' 1)"), None);
        assert_eq!(EnumType::parse("Enum8()"), None);
    }

    #[test]
    fn decoder_cases() {
        crate::data_types::cases::check(include_str!("../../tests/results/enum.jsonl"));
    }
}
