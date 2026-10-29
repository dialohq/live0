use compiler::Fingerprint;
use types::Watermark;

use crate::Error;

/// Where a subscriber is, so it can resume the same query from there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    pub fingerprint: Fingerprint,
    pub watermark: Watermark,
}

impl Cursor {
    /// An opaque, URL-safe string.
    pub fn encode(&self) -> String {
        todo!()
    }

    pub fn decode(encoded: &str) -> Result<Cursor, Error> {
        todo!()
    }
}
