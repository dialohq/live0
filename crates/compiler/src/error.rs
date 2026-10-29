use parser::error::Span;

/// Why a query can't be compiled, pointing at the part of the query that's at fault when there is one.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct Error {
    pub message: String,
    pub span: Option<Span>,
}

impl Error {
    /// Renders the error against the query's source, like `parser::error::SyntaxError::format`.
    pub fn format(&self, src: &str) -> String {
        todo!()
    }
}
