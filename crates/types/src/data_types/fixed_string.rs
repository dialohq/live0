use serde_json::Value as Json;

use crate::data_types::HistoryError;

/// Exactly `len` bytes; shorter strings are padded with `\0`.
#[derive(Debug, Clone, PartialEq)]
pub struct FixedStringType {
    pub(crate) len: usize,
}

impl FixedStringType {
    /// Only strings of at most `len` bytes; unlike `String`, numbers and booleans are rejected.
    pub(crate) fn decode_event(&self, json: Option<&Json>) -> Option<String> {
        match json {
            None | Some(Json::Null) => Some("\0".repeat(self.len)),
            Some(Json::String(s)) if s.len() <= self.len => Some(self.pad(s)),
            Some(_) => None,
        }
    }

    pub(crate) fn decode_history(&self, json: &Json) -> Result<String, HistoryError> {
        json.as_str()
            .filter(|s| s.len() == self.len)
            .map(str::to_owned)
            .ok_or_else(|| HistoryError::new(format!("FixedString({})", self.len), json))
    }

    pub(crate) fn parse(raw: &str) -> Option<Self> {
        let len = raw
            .strip_prefix("FixedString(")?
            .strip_suffix(')')?
            .trim()
            .parse()
            .ok()?;
        (len > 0).then_some(Self { len })
    }

    fn pad(&self, s: &str) -> String {
        let mut padded = String::with_capacity(self.len);
        padded.push_str(s);
        padded.extend(std::iter::repeat_n('\0', self.len - s.len()));
        padded
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn decoder_cases() {
        crate::data_types::cases::check(include_str!("../../tests/results/fixed_string.jsonl"));
    }
}
