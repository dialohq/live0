use std::path::PathBuf;

use crate::Config;

/// Environment variables are read as `LIVE0_<SECTION>__<KEY>`, e.g. `LIVE0_CLICKHOUSE__URL`.
pub const ENV_PREFIX: &str = "LIVE0_";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("can't read config file {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{origin}: {message}")]
    Invalid { origin: String, message: String },
}

/// Where the effective config comes from; earlier fields take precedence, then defaults.
#[derive(Debug, Clone, Default)]
pub struct Sources {
    /// Dotted keys, e.g. `("server.listen", "0.0.0.0:8080")`, from a binary's flags.
    pub explicit: Vec<(String, String)>,
    pub env: Vec<(String, String)>,
    pub file: Option<PathBuf>,
}

impl Sources {
    /// Takes the process environment variables that start with [`ENV_PREFIX`].
    pub fn from_process(file: Option<PathBuf>, explicit: Vec<(String, String)>) -> Self {
        todo!()
    }
}

/// Merges the sources and checks the result on its own, e.g. duplicate table names.
pub fn load(sources: &Sources) -> Result<Config, Error> {
    todo!()
}
