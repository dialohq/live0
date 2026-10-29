use functions::EvalError;
use types::{Row, Value};

/// An expression lowered to read its source row by position, evaluated as ClickHouse would.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Field(usize),
    Literal(Value),
    Not(Box<Expr>),
    And(Vec<Expr>),
    Or(Vec<Expr>),
    Compare {
        op: CompareOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    IsNull {
        expr: Box<Expr>,
        negated: bool,
    },
    InList {
        expr: Box<Expr>,
        list: Vec<Value>,
        negated: bool,
    },
    Call {
        function: Function,
        args: Vec<Expr>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
}

/// The scalar functions that can be evaluated live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Function {}

impl Expr {
    pub fn eval(&self, row: &Row) -> Result<Value, EvalError> {
        todo!()
    }

    /// Whether a filter keeps the row; `NULL` doesn't.
    pub fn matches(&self, row: &Row) -> Result<bool, EvalError> {
        todo!()
    }
}
