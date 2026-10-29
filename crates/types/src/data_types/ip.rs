use std::net::{Ipv4Addr, Ipv6Addr};

use serde_json::Value as Json;

use crate::data_types::HistoryError;

#[derive(Debug, Clone, PartialEq)]
pub struct Ipv4Type;

#[derive(Debug, Clone, PartialEq)]
pub struct Ipv6Type;

impl Ipv4Type {
    /// Dotted quads only; octets may have leading zeros.
    pub(crate) fn decode_event(&self, json: Option<&Json>) -> Option<Ipv4Addr> {
        match json {
            None | Some(Json::Null) => Some(Ipv4Addr::UNSPECIFIED),
            Some(Json::String(s)) => parse_v4(s, true).map(Ipv4Addr::from),
            Some(_) => None,
        }
    }

    pub(crate) fn decode_history(&self, json: &Json) -> Result<Ipv4Addr, HistoryError> {
        json.as_str()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| HistoryError::new("IPv4", json))
    }
}

impl Ipv6Type {
    /// An IPv6 address as ClickHouse reads it, or a plain IPv4 one, which is mapped into `::ffff:0:0/96`.
    pub(crate) fn decode_event(&self, json: Option<&Json>) -> Option<Ipv6Addr> {
        match json {
            None | Some(Json::Null) => Some(Ipv6Addr::UNSPECIFIED),
            Some(Json::String(s)) if !s.contains(':') => {
                parse_v4(s, false).map(|v4| Ipv4Addr::from(v4).to_ipv6_mapped())
            }
            Some(Json::String(s)) => parse_v6(s).map(Ipv6Addr::from),
            Some(_) => None,
        }
    }

    pub(crate) fn decode_history(&self, json: &Json) -> Result<Ipv6Addr, HistoryError> {
        json.as_str()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| HistoryError::new("IPv6", json))
    }
}

/// Four decimal octets up to 255, which may have leading zeros, except the first one unless `padded_first`.
fn parse_v4(s: &str, padded_first: bool) -> Option<u32> {
    let mut parts = s.split('.');
    let mut n = 0u32;
    for i in 0..4 {
        let part = parts.next()?;
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        if i == 0 && !padded_first && part.len() > 1 && part.starts_with('0') {
            return None;
        }
        let digits = part.trim_start_matches('0');
        let octet: u32 = if digits.is_empty() {
            0
        } else {
            digits.parse().ok()?
        };
        if octet > 255 {
            return None;
        }
        n = n << 8 | octet;
    }
    parts.next().is_none().then_some(n)
}

/// Groups of 1–4 hex digits. Unlike RFC 4291, `::` may stand for no groups, a run of three colons
/// between groups works as `::`, and one trailing colon is ignored before the 8th group. An IPv4 address may end it.
fn parse_v6(s: &str) -> Option<u128> {
    let b = s.as_bytes();
    let colons = |from: usize| b[from..].iter().take_while(|c| **c == b':').count();
    let (mut head, mut tail) = (Vec::new(), Vec::new());
    let mut compressed = match colons(0) {
        0 => false,
        2 => true,
        _ => return None,
    };
    let mut i = if compressed { 2 } else { 0 };
    while i < b.len() {
        let end = s[i..].find(':').map_or(b.len(), |e| i + e);
        let token = &s[i..end];
        let groups = if compressed { &mut tail } else { &mut head };
        if token.contains('.') {
            if end != b.len() {
                return None;
            }
            let v4 = parse_v4(token, true)?;
            groups.extend([(v4 >> 16) as u16, v4 as u16]);
        } else if (1..=4).contains(&token.len()) && token.bytes().all(|c| c.is_ascii_hexdigit()) {
            groups.push(u16::from_str_radix(token, 16).ok()?);
        } else {
            return None;
        }
        let run = colons(end);
        if run > 0 && head.len() + tail.len() == 8 {
            return None;
        }
        i = end + run;
        match run {
            0 => break,
            1 if i == b.len() => break,
            1 => {}
            2 | 3 if !compressed => compressed = true,
            _ => return None,
        }
    }
    let total = head.len() + tail.len();
    if total > 8 || (!compressed && total != 8) {
        return None;
    }
    let zeros = std::iter::repeat_n(0u16, 8 - total);
    let n = head
        .into_iter()
        .chain(zeros)
        .chain(tail)
        .fold(0u128, |n, g| n << 16 | u128::from(g));
    Some(n)
}

#[cfg(test)]
mod tests {
    #[test]
    fn decoder_cases() {
        crate::data_types::cases::check(include_str!("../../tests/results/ip.jsonl"));
    }
}
