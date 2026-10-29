use types::{DataType, Value};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("unknown aggregate function `{0}`")]
    UnknownAggregate(String),
    #[error("`{function}` doesn't take arguments {args:?}")]
    Arguments {
        function: &'static str,
        args: Vec<DataType>,
    },
}

/// A value that doesn't fit the types the function was bound to.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EvalError {
    #[error("`{function}` got {value:?}")]
    Mismatch {
        function: &'static str,
        value: Value,
    },
}
