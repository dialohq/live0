use crate::Error;

/// A query's state at its watermark, for shapes that allow saving one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkpoint {
    bytes: Vec<u8>,
}

impl Checkpoint {
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, Error> {
        todo!()
    }
}
