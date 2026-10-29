use compiler::{EvalError, Fingerprint};

use crate::FetchId;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("answer for unknown fetch {0:?}")]
    UnknownFetch(FetchId),
    #[error("checkpoint is for plan {checkpoint}, not {plan}")]
    CheckpointMismatch {
        checkpoint: Fingerprint,
        plan: Fingerprint,
    },
    #[error("malformed checkpoint: {0}")]
    MalformedCheckpoint(String),
    #[error(transparent)]
    Eval(#[from] EvalError),
}
