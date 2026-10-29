/// Messages rather than sources, because errors from a remote manager arrive over gRPC as text.
#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error("no table `{0}` to snapshot")]
    UnknownTable(String),
    #[error("pause server: {0}")]
    Pause(String),
    #[error("cloning: {0}")]
    Clone(String),
    #[error("snapshot manager: {0}")]
    Remote(String),
    #[error("lease expired")]
    LeaseExpired,
}
