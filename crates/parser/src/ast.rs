use crate::error::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub projection: Projection,                   // SELECT ...
    pub from: TableRef,                           // FROM ...
    pub predicate: Option<BoolExpr>,              // WHERE ...
    pub group_by: Option<(Vec<ColumnRef>, Span)>, // GROUP BY ...
    pub order: Option<Order>,                     // ORDER BY ...
    pub limit: Option<(u64, Span)>,               // LIMIT ...
    pub offset: Option<(u64, Span)>,              // OFFSET ...
}

#[derive(Debug, Clone, PartialEq)]
pub enum Projection {
    Wildcard,
    Items(Vec<SelectItem>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SelectItem {
    pub expr: Expr,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Column(ColumnRef),
    Literal(Literal),
    Call {
        name: String,
        arg: Option<CallArg>,
        span: Span,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum CallArg {
    Star,
    Column(ColumnRef),
    Literal(Literal),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColumnRef {
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableRef {
    pub name: String,
    pub final_span: Option<Span>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BoolExpr {
    Expr(Expr),
    Compare {
        op: CmpOp,
        left: Box<BoolExpr>,
        right: Box<BoolExpr>,
        span: Span,
    },
    And(Box<BoolExpr>, Box<BoolExpr>),
    Or(Box<BoolExpr>, Box<BoolExpr>),
    Not {
        expr: Box<BoolExpr>,
        span: Span,
    },
    IsNull {
        expr: Box<BoolExpr>,
        negated: bool,
        span: Span,
    },
    InList {
        expr: Box<BoolExpr>,
        list: Vec<Literal>,
        negated: bool,
        span: Span,
    },
    Exists {
        query: Box<Query>,
        negated: bool,
        span: Span,
    },
    InSubquery {
        expr: Box<BoolExpr>,
        query: Box<Query>,
        negated: bool,
        span: Span,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Order {
    All {
        direction: Option<Direction>,
        nulls: Option<NullsOrder>,
        span: Span,
    },
    Items(Vec<OrderItem>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct OrderItem {
    pub expr: Expr,
    pub direction: Option<Direction>,
    pub nulls: Option<NullsOrder>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NullsOrder {
    First,
    Last,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Literal {
    pub value: LiteralValue,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LiteralValue {
    Null,
    String(String),
    Numeric(String),
    Bool(bool),
    Param { name: String, ty: String },
}
