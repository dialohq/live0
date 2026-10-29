use parser::error::SyntaxError;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Syntax(#[from] SyntaxError),
    #[error(transparent)]
    Catalog(#[from] catalog::Error),
    #[error(transparent)]
    Compile(#[from] compiler::Error),
    #[error(transparent)]
    State(#[from] qstate::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Snapshot(#[from] snapshot::Error),
    #[error("invalid cursor: {0}")]
    Cursor(String),
}

impl Error {
    /// Renders the error against the query's source, pointing at the fault where there's a span.
    pub fn format(&self, src: &str) -> String {
        todo!()
    }
}
