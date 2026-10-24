use crate::error::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub projection: Projection,                   // SELECT ...
    pub from: TableRef,                           // FROM ...
    pub predicate: Option<Expr>,                  // WHERE ...
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
        args: Vec<CallArg>,
        span: Span,
    },
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
        span: Span,
    },
    Binary {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
        span: Span,
    },
    IsNull {
        expr: Box<Expr>,
        negated: bool,
        span: Span,
    },
    InList {
        expr: Box<Expr>,
        list: Vec<Expr>,
        negated: bool,
        span: Span,
    },
    InSubquery {
        expr: Box<Expr>,
        query: Box<Query>,
        negated: bool,
        span: Span,
    },
    Exists {
        query: Box<Query>,
        negated: bool,
        span: Span,
    },
    Subquery {
        query: Box<Query>,
        span: Span,
    },
    Like {
        expr: Box<Expr>,
        pattern: Box<Expr>,
        negated: bool,
        case_insensitive: bool,
        span: Span,
    },
    Between {
        expr: Box<Expr>,
        low: Box<Expr>,
        high: Box<Expr>,
        negated: bool,
        span: Span,
    },
}

impl Expr {
    pub fn span(&self) -> Span {
        let join = |a: Span, b: Span| Span(a.0.min(b.0), a.1.max(b.1));
        match self {
            Expr::Column(c) => c.span,
            Expr::Literal(l) => l.span,
            Expr::Call { args, span, .. } => args.iter().fold(*span, |s, a| match a {
                CallArg::Star => s,
                CallArg::Expr(e) => join(s, e.span()),
            }),
            Expr::Unary { expr, span, .. } => join(*span, expr.span()),
            Expr::Binary {
                left, right, span, ..
            } => join(join(left.span(), *span), right.span()),
            Expr::IsNull { expr, span, .. } => join(expr.span(), *span),
            Expr::InList {
                expr, list, span, ..
            } => list
                .iter()
                .fold(join(expr.span(), *span), |s, e| join(s, e.span())),
            Expr::InSubquery { expr, span, .. } => join(expr.span(), *span),
            Expr::Exists { span, .. } | Expr::Subquery { span, .. } => *span,
            Expr::Like {
                expr,
                pattern,
                span,
                ..
            } => join(join(expr.span(), *span), pattern.span()),
            Expr::Between {
                expr, high, span, ..
            } => join(join(expr.span(), *span), high.span()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CallArg {
    Star,
    Expr(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColumnRef {
    pub qualifier: Option<String>,
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableRef {
    pub name: String,
    pub final_span: Option<Span>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Not,
    Neg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    And,
    Or,
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    Add,
    Sub,
    Mul,
    Div,
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
