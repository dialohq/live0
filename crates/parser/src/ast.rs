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

impl Query {
    /// Every table the query reads, its own and its subqueries', each once.
    pub fn tables(&self) -> Vec<&str> {
        let mut tables = Vec::new();
        self.collect_tables(&mut tables);
        tables
    }

    fn collect_tables<'a>(&'a self, tables: &mut Vec<&'a str>) {
        if !tables.contains(&self.from.name.as_str()) {
            tables.push(&self.from.name);
        }
        let items = match &self.projection {
            Projection::Wildcard => &[][..],
            Projection::Items(items) => items,
        };
        let orders = match &self.order {
            Some(Order::Items(items)) => &items[..],
            _ => &[],
        };
        items
            .iter()
            .map(|item| &item.expr)
            .chain(&self.predicate)
            .chain(orders.iter().map(|item| &item.expr))
            .for_each(|expr| expr.collect_tables(tables));
    }
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
    fn collect_tables<'a>(&'a self, tables: &mut Vec<&'a str>) {
        match self {
            Expr::Column(_) | Expr::Literal(_) => {}
            Expr::Call { args, .. } => args.iter().for_each(|arg| match arg {
                CallArg::Star => {}
                CallArg::Expr(e) => e.collect_tables(tables),
            }),
            Expr::Unary { expr, .. } | Expr::IsNull { expr, .. } => expr.collect_tables(tables),
            Expr::Binary { left, right, .. } => {
                left.collect_tables(tables);
                right.collect_tables(tables);
            }
            Expr::InList { expr, list, .. } => {
                expr.collect_tables(tables);
                list.iter().for_each(|e| e.collect_tables(tables));
            }
            Expr::InSubquery { expr, query, .. } => {
                expr.collect_tables(tables);
                query.collect_tables(tables);
            }
            Expr::Exists { query, .. } | Expr::Subquery { query, .. } => {
                query.collect_tables(tables);
            }
            Expr::Like { expr, pattern, .. } => {
                expr.collect_tables(tables);
                pattern.collect_tables(tables);
            }
            Expr::Between {
                expr, low, high, ..
            } => {
                expr.collect_tables(tables);
                low.collect_tables(tables);
                high.collect_tables(tables);
            }
        }
    }

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

#[cfg(test)]
mod tests {
    use crate::parse;

    #[test]
    fn tables_include_subqueries_once_in_order() {
        let tables = |sql: &str| {
            parse(sql)
                .unwrap()
                .tables()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        assert_eq!(tables("SELECT * FROM events"), ["events"]);
        assert_eq!(
            tables(
                "SELECT id FROM events \
                 WHERE user IN (SELECT id FROM users) \
                 AND EXISTS (SELECT * FROM bans WHERE id IN (SELECT id FROM users)) \
                 AND id IN (SELECT id FROM events) \
                 AND n IN (SELECT n FROM totals)"
            ),
            ["events", "users", "bans", "totals"]
        );
    }
}
