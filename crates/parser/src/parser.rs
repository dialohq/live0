use crate::ast::{
    BinOp, CallArg, ColumnRef, Direction, Expr, Literal, LiteralValue, NullsOrder, Order,
    OrderItem, Projection, Query, SelectItem, TableRef, UnaryOp,
};
use crate::error::{Span, SyntaxError};
use crate::lexer::{Token, TokenKind};

const OR: u8 = 1;
const AND: u8 = 2;
const NOT: u8 = 3;
const CMP: u8 = 4;
const SUM: u8 = 5;
const PRODUCT: u8 = 6;
const NEG: u8 = 7;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Infix {
    Binary(BinOp),
    IsNull,
    In,
    Like,
    ILike,
    Between,
}

pub fn parse(tokens: Vec<Token>) -> Result<Query, SyntaxError> {
    let mut p = Parser { tokens, pos: 0 };
    let query = p.query()?;
    Ok(query)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn lookahead(&self) -> Option<&TokenKind> {
        self.tokens.get(self.pos + 1).map(|t| &t.kind)
    }

    fn next(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        if t.kind != TokenKind::Eof {
            self.pos += 1;
        }
        t
    }

    fn expected(&self, what: &str) -> SyntaxError {
        let t = self.peek();
        SyntaxError::new(
            format!("expected {what}, found {}", t.kind.describe()),
            t.span,
        )
    }

    fn keyword(&mut self, kw: &'static str) -> Result<(), SyntaxError> {
        if self.peek().kind == TokenKind::Keyword(kw) {
            self.next();
            Ok(())
        } else {
            Err(self.expected(kw))
        }
    }

    fn query(&mut self) -> Result<Query, SyntaxError> {
        let query = self.query_body()?;
        self.end()?;
        Ok(query)
    }

    fn query_body(&mut self) -> Result<Query, SyntaxError> {
        self.keyword("SELECT")?;
        let projection = self.projection()?;
        self.keyword("FROM")?;
        let from = self.table()?;
        let predicate = if self.peek().kind == TokenKind::Keyword("WHERE") {
            self.next();
            Some(self.condition()?)
        } else {
            None
        };
        let group_by = if self.peek().kind == TokenKind::Keyword("GROUP") {
            let span = self.next().span;
            self.keyword("BY")?;
            let mut keys = vec![self.group_key()?];
            while self.peek().kind == TokenKind::Comma {
                self.next();
                keys.push(self.group_key()?);
            }
            Some((keys, span))
        } else {
            None
        };
        let order = if self.peek().kind == TokenKind::Keyword("ORDER") {
            self.next();
            self.keyword("BY")?;
            Some(self.order()?)
        } else {
            None
        };
        let (mut limit, mut offset) = (None, None);
        if self.peek().kind == TokenKind::Keyword("LIMIT") {
            self.next();
            let first = self.limit()?;
            if self.peek().kind == TokenKind::Comma {
                self.next();
                offset = Some(first);
                limit = Some(self.limit()?);
            } else {
                limit = Some(first);
            }
        }
        if offset.is_none() && self.at_word("OFFSET") {
            self.next();
            offset = Some(self.count("OFFSET")?);
        }
        Ok(Query {
            projection,
            from,
            predicate,
            group_by,
            order,
            limit,
            offset,
        })
    }

    fn projection(&mut self) -> Result<Projection, SyntaxError> {
        if self.peek().kind == TokenKind::Star {
            self.next();
            if self.peek().kind == TokenKind::Comma {
                return Err(SyntaxError::new(
                    "cannot mix * with column names",
                    self.peek().span,
                ));
            }
            return Ok(Projection::Wildcard);
        }
        let mut items = vec![self.select_item()?];
        while self.peek().kind == TokenKind::Comma {
            self.next();
            if self.peek().kind == TokenKind::Star {
                return Err(SyntaxError::new(
                    "cannot mix * with column names",
                    self.peek().span,
                ));
            }
            items.push(self.select_item()?);
        }
        Ok(Projection::Items(items))
    }

    fn select_item(&mut self) -> Result<SelectItem, SyntaxError> {
        if !self.starts_expr() {
            return Err(self.expected(
                if self.pos > 0 && self.tokens[self.pos - 1].kind == TokenKind::Comma {
                    "a column name"
                } else {
                    "`*` or a column name"
                },
            ));
        }
        let expr = self.expr()?;

        if !matches!(expr, Expr::Column(_) | Expr::Literal(_) | Expr::Call { .. }) {
            return Err(SyntaxError::new(
                "expressions are not supported yet; list column names",
                expr.span(),
            ));
        }

        check_supported(&expr)?;
        self.alias_for(expr)
    }

    fn alias_for(&mut self, expr: Expr) -> Result<SelectItem, SyntaxError> {
        let alias = if self.peek().kind == TokenKind::Keyword("AS") {
            self.next();
            match self.peek().kind.clone() {
                TokenKind::Ident { name: a, .. } => {
                    self.next();
                    Some(a)
                }
                _ => return Err(self.expected("an alias")),
            }
        } else {
            None
        };
        Ok(SelectItem { expr, alias })
    }

    fn condition(&mut self) -> Result<Expr, SyntaxError> {
        let expr = self.expr()?;
        check_supported(&expr)?;
        Ok(expr)
    }

    fn group_key(&mut self) -> Result<ColumnRef, SyntaxError> {
        let t = self.peek().clone();
        if self.at_word("ALL") {
            return Err(SyntaxError::new(
                "GROUP BY ALL is not supported yet; name the columns",
                t.span,
            ));
        }
        if !self.starts_expr() {
            return Err(self.expected("a column name"));
        }
        let expr = self.expr()?;
        check_supported(&expr)?;
        match expr {
            Expr::Column(col) => Ok(col),
            Expr::Literal(Literal {
                value: LiteralValue::Numeric(_),
                span,
            }) => Err(SyntaxError::new(
                "grouping by position is not supported yet; name the column",
                span,
            )),
            _ => Err(SyntaxError::new(
                "grouping by expressions is not supported yet; name a column",
                expr.span(),
            )),
        }
    }

    fn order(&mut self) -> Result<Order, SyntaxError> {
        if self.at_word("ALL") {
            let span = self.next().span;
            let (direction, nulls) = self.order_modifiers()?;
            if self.peek().kind == TokenKind::Comma {
                return Err(SyntaxError::new(
                    "ORDER BY ALL can't be combined with other columns",
                    self.peek().span,
                ));
            }
            return Ok(Order::All {
                direction,
                nulls,
                span,
            });
        }
        let mut items = vec![self.order_item()?];
        while self.peek().kind == TokenKind::Comma {
            self.next();
            items.push(self.order_item()?);
        }
        Ok(Order::Items(items))
    }

    fn order_item(&mut self) -> Result<OrderItem, SyntaxError> {
        if !self.starts_expr() {
            return Err(self.expected("a column name"));
        }
        let expr = self.expr()?;
        match &expr {
            Expr::Literal(Literal {
                value: LiteralValue::Numeric(_),
                span,
            }) => {
                return Err(SyntaxError::new(
                    "ordering by position is not supported yet; name the column",
                    *span,
                ));
            }
            Expr::Column(_) | Expr::Call { .. } => check_supported(&expr)?,
            _ => {
                return Err(SyntaxError::new(
                    "ordering by expressions is not supported yet; name a column",
                    expr.span(),
                ));
            }
        }
        let (direction, nulls) = self.order_modifiers()?;
        Ok(OrderItem {
            expr,
            direction,
            nulls,
        })
    }

    fn order_modifiers(&mut self) -> Result<(Option<Direction>, Option<NullsOrder>), SyntaxError> {
        let direction = if self.at_word("ASC") {
            self.next();
            Some(Direction::Asc)
        } else if self.at_word("DESC") {
            self.next();
            Some(Direction::Desc)
        } else {
            None
        };
        let nulls = if self.at_word("NULLS") {
            self.next();
            if self.at_word("FIRST") {
                self.next();
                Some(NullsOrder::First)
            } else if self.at_word("LAST") {
                self.next();
                Some(NullsOrder::Last)
            } else {
                return Err(self.expected("FIRST or LAST"));
            }
        } else {
            None
        };
        for word in ["COLLATE", "WITH"] {
            if self.at_word(word) {
                return Err(SyntaxError::new(
                    format!("{word} in ORDER BY is not supported"),
                    self.peek().span,
                ));
            }
        }
        Ok((direction, nulls))
    }

    fn count(&mut self, what: &str) -> Result<(u64, Span), SyntaxError> {
        let t = self.peek().clone();
        let value = match &t.kind {
            TokenKind::Number(n) => n.parse::<u64>().ok(),
            _ => None,
        };
        let Some(value) = value else {
            return Err(SyntaxError::new(
                format!("{what} needs a non-negative whole number"),
                t.span,
            ));
        };
        self.next();
        Ok((value, t.span))
    }

    fn limit(&mut self) -> Result<(u64, Span), SyntaxError> {
        let limit = self.count("LIMIT")?;
        let next = self.peek().clone();
        if self.at_word("WITH") {
            return Err(SyntaxError::new(
                "LIMIT … WITH TIES is not supported yet",
                next.span,
            ));
        }
        if next.kind == TokenKind::Keyword("BY") {
            return Err(SyntaxError::new(
                "LIMIT … BY is not supported yet",
                next.span,
            ));
        }
        Ok(limit)
    }

    fn at_word(&self, word: &str) -> bool {
        matches!(
            &self.peek().kind,
            TokenKind::Ident { name, quoted: false } if name.eq_ignore_ascii_case(word)
        )
    }

    fn starts_expr(&self) -> bool {
        matches!(
            self.peek().kind,
            TokenKind::Ident { .. }
                | TokenKind::Number(_)
                | TokenKind::String(_)
                | TokenKind::Param { .. }
                | TokenKind::Minus
                | TokenKind::LParen
                | TokenKind::Keyword("TRUE" | "FALSE" | "NULL" | "NOT" | "EXISTS")
        )
    }

    /// Any expression, supported or not; the caller checks what it got.
    fn expr(&mut self) -> Result<Expr, SyntaxError> {
        self.expr_bp(0)
    }

    /// Precedence climbing: parses operators that bind tighter than `min_bp`.
    fn expr_bp(&mut self, min_bp: u8) -> Result<Expr, SyntaxError> {
        let mut left = self.prefix()?;
        loop {
            // `NOT` is an infix operator only in front of IN, LIKE, ILIKE and BETWEEN.
            let negated = self.peek().kind == TokenKind::Keyword("NOT")
                && matches!(
                    self.lookahead(),
                    Some(TokenKind::Keyword("IN" | "LIKE" | "ILIKE" | "BETWEEN"))
                );
            let op = if negated {
                self.tokens[self.pos + 1].clone()
            } else {
                self.peek().clone()
            };
            let (bp, infix) = match op.kind {
                TokenKind::Keyword("OR") => (OR, Infix::Binary(BinOp::Or)),
                TokenKind::Keyword("AND") => (AND, Infix::Binary(BinOp::And)),
                TokenKind::Eq => (CMP, Infix::Binary(BinOp::Eq)),
                TokenKind::NotEq => (CMP, Infix::Binary(BinOp::NotEq)),
                TokenKind::Lt => (CMP, Infix::Binary(BinOp::Lt)),
                TokenKind::LtEq => (CMP, Infix::Binary(BinOp::LtEq)),
                TokenKind::Gt => (CMP, Infix::Binary(BinOp::Gt)),
                TokenKind::GtEq => (CMP, Infix::Binary(BinOp::GtEq)),
                TokenKind::Keyword("IS") => (CMP, Infix::IsNull),
                TokenKind::Keyword("IN") => (CMP, Infix::In),
                TokenKind::Keyword("LIKE") => (CMP, Infix::Like),
                TokenKind::Keyword("ILIKE") => (CMP, Infix::ILike),
                TokenKind::Keyword("BETWEEN") => (CMP, Infix::Between),
                TokenKind::Plus => (SUM, Infix::Binary(BinOp::Add)),
                TokenKind::Minus => (SUM, Infix::Binary(BinOp::Sub)),
                TokenKind::Star => (PRODUCT, Infix::Binary(BinOp::Mul)),
                TokenKind::Slash => (PRODUCT, Infix::Binary(BinOp::Div)),
                _ => break,
            };
            if bp <= min_bp {
                break;
            }
            if negated {
                self.next();
            }
            self.next();
            let span = op.span;
            left = match infix {
                Infix::Binary(op) => {
                    let right = self.expr_bp(bp)?;
                    Expr::Binary {
                        op,
                        left: Box::new(left),
                        right: Box::new(right),
                        span,
                    }
                }
                Infix::IsNull => {
                    let negated = self.peek().kind == TokenKind::Keyword("NOT");
                    if negated {
                        self.next();
                    }
                    if self.peek().kind != TokenKind::Keyword("NULL") {
                        return Err(self.expected("NULL"));
                    }
                    self.next();
                    Expr::IsNull {
                        expr: Box::new(left),
                        negated,
                        span,
                    }
                }
                Infix::In => self.in_rest(left, negated, span)?,
                Infix::Like | Infix::ILike => Expr::Like {
                    expr: Box::new(left),
                    pattern: Box::new(self.expr_bp(CMP)?),
                    negated,
                    case_insensitive: infix == Infix::ILike,
                    span,
                },
                Infix::Between => {
                    let low = self.expr_bp(CMP)?;
                    self.keyword("AND")?;
                    let high = self.expr_bp(CMP)?;
                    Expr::Between {
                        expr: Box::new(left),
                        low: Box::new(low),
                        high: Box::new(high),
                        negated,
                        span,
                    }
                }
            };
        }
        Ok(left)
    }

    /// What can start an expression: values, columns, calls, `( … )`, `NOT`, unary `-` and
    /// `[NOT] EXISTS`.
    fn prefix(&mut self) -> Result<Expr, SyntaxError> {
        let t = self.peek().clone();
        match &t.kind {
            TokenKind::Number(_)
            | TokenKind::String(_)
            | TokenKind::Keyword("TRUE" | "FALSE" | "NULL")
            | TokenKind::Param { .. } => Ok(Expr::Literal(self.literal()?)),
            TokenKind::Minus if matches!(self.lookahead(), Some(TokenKind::Number(_))) => {
                Ok(Expr::Literal(self.literal()?))
            }
            TokenKind::Minus => {
                self.next();
                let expr = self.expr_bp(NEG)?;
                Ok(Expr::Unary {
                    op: UnaryOp::Neg,
                    expr: Box::new(expr),
                    span: t.span,
                })
            }
            TokenKind::Keyword("NOT")
                if self.lookahead() == Some(&TokenKind::Keyword("EXISTS")) =>
            {
                self.next();
                self.exists(true)
            }
            TokenKind::Keyword("NOT") => {
                self.next();
                let expr = self.expr_bp(NOT)?;
                Ok(Expr::Unary {
                    op: UnaryOp::Not,
                    expr: Box::new(expr),
                    span: t.span,
                })
            }
            TokenKind::Keyword("EXISTS") => self.exists(false),
            TokenKind::LParen => {
                self.next();
                if self.peek().kind == TokenKind::Keyword("SELECT") {
                    let span = self.peek().span;
                    let query = self.query_body()?;
                    if self.peek().kind != TokenKind::RParen {
                        return Err(self.expected("`)` after the sub-query"));
                    }
                    self.next();
                    return Ok(Expr::Subquery {
                        query: Box::new(query),
                        span,
                    });
                }
                let inner = self.expr()?;
                if self.peek().kind != TokenKind::RParen {
                    return Err(self.expected("`)`"));
                }
                self.next();
                Ok(inner)
            }
            TokenKind::Ident { name, .. } if self.lookahead() == Some(&TokenKind::LParen) => {
                self.call(name.clone(), t.span)
            }
            TokenKind::Ident { .. } => Ok(Expr::Column(self.column()?)),
            _ => Err(self.expected("an expression")),
        }
    }

    /// At the function name, followed by `(`.
    fn call(&mut self, name: String, span: Span) -> Result<Expr, SyntaxError> {
        self.next();
        self.next();
        let mut args = Vec::new();
        if self.peek().kind != TokenKind::RParen {
            loop {
                if self.peek().kind == TokenKind::Star {
                    self.next();
                    args.push(CallArg::Star);
                } else {
                    args.push(CallArg::Expr(self.expr()?));
                }
                if self.peek().kind != TokenKind::Comma {
                    break;
                }
                self.next();
            }
        }
        if self.peek().kind != TokenKind::RParen {
            return Err(self.expected("`)`"));
        }
        self.next();
        Ok(Expr::Call { name, args, span })
    }

    /// `name` or `qualifier.name`.
    fn column(&mut self) -> Result<ColumnRef, SyntaxError> {
        let t = self.peek().clone();
        let TokenKind::Ident { name, .. } = t.kind else {
            return Err(self.expected("a column name"));
        };
        self.next();
        if self.peek().kind != TokenKind::Dot {
            return Ok(ColumnRef {
                qualifier: None,
                name,
                span: t.span,
            });
        }
        self.next();
        let n = self.peek().clone();
        let TokenKind::Ident { name: field, .. } = n.kind else {
            return Err(self.expected("a column name after `.`"));
        };
        self.next();
        Ok(ColumnRef {
            qualifier: Some(name),
            name: field,
            span: Span(t.span.0, n.span.1),
        })
    }

    /// After `[NOT] IN`.
    fn in_rest(&mut self, expr: Expr, negated: bool, span: Span) -> Result<Expr, SyntaxError> {
        if self.peek().kind != TokenKind::LParen {
            return Err(self.expected("`(`"));
        }
        self.next();
        if self.peek().kind == TokenKind::Keyword("SELECT") {
            let query = self.query_body()?;
            if self.peek().kind != TokenKind::RParen {
                return Err(self.expected("`)` after the sub-query"));
            }
            self.next();
            return Ok(Expr::InSubquery {
                expr: Box::new(expr),
                query: Box::new(query),
                negated,
                span,
            });
        }
        let mut list = vec![self.expr()?];
        while self.peek().kind == TokenKind::Comma {
            self.next();
            list.push(self.expr()?);
        }
        if self.peek().kind != TokenKind::RParen {
            return Err(self.expected("`,` or `)`"));
        }
        self.next();
        Ok(Expr::InList {
            expr: Box::new(expr),
            list,
            negated,
            span,
        })
    }

    fn literal(&mut self) -> Result<Literal, SyntaxError> {
        let t = self.peek().clone();
        let value = match t.kind {
            TokenKind::Number(n) => LiteralValue::Numeric(n),
            TokenKind::String(v) => LiteralValue::String(v),
            TokenKind::Keyword("TRUE") => LiteralValue::Bool(true),
            TokenKind::Keyword("FALSE") => LiteralValue::Bool(false),
            TokenKind::Keyword("NULL") => LiteralValue::Null,
            TokenKind::Param { name, ty } => LiteralValue::Param { name, ty },
            TokenKind::Minus => {
                self.next();
                let n = self.peek().clone();
                let TokenKind::Number(num) = n.kind else {
                    return Err(self.expected("a number after `-`"));
                };
                self.next();
                return Ok(Literal {
                    value: LiteralValue::Numeric(format!("-{num}")),
                    span: Span(t.span.0, n.span.1),
                });
            }
            _ => return Err(self.expected("a value")),
        };
        self.next();
        Ok(Literal {
            value,
            span: t.span,
        })
    }

    /// At `EXISTS`.
    fn exists(&mut self, negated: bool) -> Result<Expr, SyntaxError> {
        let span = self.next().span;
        if self.peek().kind != TokenKind::LParen {
            return Err(self.expected("`(` and a sub-query after EXISTS"));
        }
        self.next();
        if self.peek().kind != TokenKind::Keyword("SELECT") {
            return Err(self.expected("a sub-query (SELECT …) after EXISTS"));
        }
        let query = self.query_body()?;
        if self.peek().kind != TokenKind::RParen {
            return Err(self.expected("`)` after the sub-query"));
        }
        self.next();
        Ok(Expr::Exists {
            query: Box::new(query),
            negated,
            span,
        })
    }

    fn table(&mut self) -> Result<TableRef, SyntaxError> {
        let t = self.peek().clone();
        let TokenKind::Ident { name, .. } = t.kind else {
            return Err(self.expected("a table name"));
        };
        self.next();
        let n = self.peek().clone();

        let final_span = match n.kind {
            TokenKind::Dot => {
                return Err(SyntaxError::new(
                    "qualified table names are not supported",
                    self.peek().span,
                ));
            }
            TokenKind::Keyword("AS") => {
                return Err(SyntaxError::new(
                    "table aliases are not supported yet",
                    self.peek().span,
                ));
            }
            _ if self.at_word("FINAL") => {
                self.next();
                Some(n.span)
            }
            _ => None,
        };

        Ok(TableRef {
            name,
            final_span,
            span: t.span,
        })
    }

    fn end(&mut self) -> Result<(), SyntaxError> {
        if self.peek().kind == TokenKind::Semicolon {
            self.next();
        }
        let t = self.peek().clone();
        match t.kind {
            TokenKind::Eof => Ok(()),
            TokenKind::Keyword(kw @ ("HAVING" | "JOIN" | "UNION")) => Err(SyntaxError::new(
                format!("{kw} is not supported yet"),
                t.span,
            )),
            TokenKind::Comma => Err(SyntaxError::new(
                "selecting from multiple tables is not supported",
                t.span,
            )),
            _ => Err(self.expected("end of query")),
        }
    }
}

/// Checks if expression is generally unsupported in any part of a query
fn check_supported(expr: &Expr) -> Result<(), SyntaxError> {
    match expr {
        Expr::Literal(_) | Expr::Exists { .. } => Ok(()),
        Expr::Column(c) => {
            if c.qualifier.is_some() {
                return Err(SyntaxError::new(
                    "qualified column names are not supported yet",
                    c.span,
                ));
            }
            Ok(())
        }
        Expr::Call { args, span, .. } => {
            match args.first() {
                None | Some(CallArg::Star) => {}
                Some(CallArg::Expr(arg @ Expr::Column(_))) => check_supported(arg)?,
                Some(CallArg::Expr(Expr::Call { span, .. })) => {
                    return Err(SyntaxError::new(
                        "nested functions are not supported yet",
                        *span,
                    ));
                }
                Some(CallArg::Expr(arg)) => {
                    return Err(SyntaxError::new(
                        "function arguments other than a column are not supported yet",
                        arg.span(),
                    ));
                }
            }
            if let Some(extra) = args.get(1) {
                return Err(SyntaxError::new(
                    "functions with more than one argument are not supported yet",
                    match extra {
                        CallArg::Star => *span,
                        CallArg::Expr(e) => e.span(),
                    },
                ));
            }
            Ok(())
        }
        Expr::Unary {
            op: UnaryOp::Neg,
            span,
            ..
        }
        | Expr::Binary {
            op: BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div,
            span,
            ..
        } => Err(SyntaxError::new("arithmetic is not supported yet", *span)),
        Expr::Unary { expr, .. } | Expr::IsNull { expr, .. } | Expr::InSubquery { expr, .. } => {
            check_supported(expr)
        }
        Expr::Binary { left, right, .. } => {
            check_supported(left)?;
            check_supported(right)
        }
        Expr::InList { expr, list, .. } => {
            check_supported(expr)?;
            match list.iter().find(|e| !matches!(e, Expr::Literal(_))) {
                Some(e) => Err(SyntaxError::new(
                    "only values are supported in IN lists yet",
                    e.span(),
                )),
                None => Ok(()),
            }
        }
        Expr::Subquery { span, .. } => Err(SyntaxError::new("subqueries are not supported", *span)),
        Expr::Like {
            case_insensitive,
            span,
            ..
        } => Err(SyntaxError::new(
            format!(
                "{} is not supported yet",
                if *case_insensitive { "ILIKE" } else { "LIKE" }
            ),
            *span,
        )),
        Expr::Between { span, .. } => Err(SyntaxError::new("BETWEEN is not supported yet", *span)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::tokenize;

    fn parse_sql(src: &str) -> Result<Query, SyntaxError> {
        parse(tokenize(src).expect("lexes"))
    }

    fn ok(src: &str) -> Query {
        parse_sql(src).unwrap_or_else(|e| panic!("{src:?} failed: {e}"))
    }

    fn err(src: &str) -> SyntaxError {
        match parse_sql(src) {
            Ok(q) => panic!("{src:?} parsed unexpectedly: {q:?}"),
            Err(e) => e,
        }
    }

    fn msg(src: &str) -> String {
        err(src).message
    }

    fn col(name: &str, start: usize) -> ColumnRef {
        ColumnRef {
            qualifier: None,
            name: name.into(),
            span: Span(start, start + name.len()),
        }
    }

    fn lit(value: LiteralValue, start: usize, end: usize) -> Literal {
        Literal {
            value,
            span: Span(start, end),
        }
    }

    fn num(n: &str, start: usize) -> Literal {
        lit(LiteralValue::Numeric(n.into()), start, start + n.len())
    }

    fn items(q: &Query) -> &[SelectItem] {
        match &q.projection {
            Projection::Items(items) => items,
            Projection::Wildcard => panic!("expected items, got *"),
        }
    }

    fn where_(q: &Query) -> &Expr {
        q.predicate.as_ref().expect("has WHERE")
    }

    fn order_items(q: &Query) -> &[OrderItem] {
        match q.order.as_ref().expect("has ORDER BY") {
            Order::Items(items) => items,
            Order::All { .. } => panic!("expected items, got ALL"),
        }
    }

    // ---- basic shape ----

    #[test]
    fn select_star() {
        assert_eq!(
            ok("SELECT * FROM t"),
            Query {
                projection: Projection::Wildcard,
                from: TableRef {
                    name: "t".into(),
                    final_span: None,
                    span: Span(14, 15),
                },
                predicate: None,
                group_by: None,
                order: None,
                limit: None,
                offset: None,
            }
        );
    }

    #[test]
    fn trailing_semicolon_is_allowed() {
        assert_eq!(ok("SELECT * FROM t;"), ok("SELECT * FROM t"));
    }

    #[test]
    fn keywords_are_case_insensitive() {
        assert_eq!(ok("select * from t"), ok("SELECT * FROM t"));
    }

    #[test]
    fn select_columns() {
        let q = ok("SELECT a, b FROM t");
        assert_eq!(
            items(&q),
            [
                SelectItem {
                    expr: Expr::Column(col("a", 7)),
                    alias: None
                },
                SelectItem {
                    expr: Expr::Column(col("b", 10)),
                    alias: None
                },
            ]
        );
    }

    #[test]
    fn quoted_identifiers() {
        let q = ok("SELECT \"select\" FROM `my table`");
        assert_eq!(
            items(&q)[0].expr,
            Expr::Column(ColumnRef {
                qualifier: None,
                name: "select".into(),
                span: Span(7, 15),
            })
        );
        assert_eq!(q.from.name, "my table");
    }

    #[test]
    fn unreserved_words_are_column_names() {
        let q = ok("SELECT asc, desc, nulls, first, last, offset, with FROM t");
        let names: Vec<_> = items(&q)
            .iter()
            .map(|i| match &i.expr {
                Expr::Column(c) => c.name.as_str(),
                e => panic!("expected column, got {e:?}"),
            })
            .collect();
        assert_eq!(
            names,
            ["asc", "desc", "nulls", "first", "last", "offset", "with"]
        );
    }

    #[test]
    fn final_and_all_are_column_names() {
        let q = ok("SELECT final, all FROM t WHERE all = 1 ORDER BY final");
        assert_eq!(items(&q)[0].expr, Expr::Column(col("final", 7)));
        assert_eq!(items(&q)[1].expr, Expr::Column(col("all", 14)));
        assert_eq!(order_items(&q)[0].expr, Expr::Column(col("final", 48)));
    }

    #[test]
    fn quoted_words_are_never_contextual_keywords() {
        let q = ok("SELECT * FROM t ORDER BY \"all\" DESC");
        assert_eq!(
            order_items(&q)[0],
            OrderItem {
                expr: Expr::Column(ColumnRef {
                    qualifier: None,
                    name: "all".into(),
                    span: Span(25, 30),
                }),
                direction: Some(Direction::Desc),
                nulls: None,
            }
        );
        let q = ok("SELECT a FROM t GROUP BY `all`");
        assert!(matches!(
            &q.group_by.as_ref().unwrap().0[..],
            [ColumnRef { name, .. }] if name == "all"
        ));
        assert_eq!(
            msg("SELECT * FROM t \"final\""),
            "expected end of query, found identifier `final`"
        );
        assert_eq!(
            msg("SELECT * FROM t ORDER BY a \"desc\""),
            "expected end of query, found identifier `desc`"
        );
        assert_eq!(
            msg("SELECT * FROM t \"offset\" 3"),
            "expected end of query, found identifier `offset`"
        );
    }

    #[test]
    fn aliases() {
        let q = ok("SELECT a AS x, count(*) AS n FROM t");
        assert_eq!(items(&q)[0].alias.as_deref(), Some("x"));
        assert_eq!(items(&q)[1].alias.as_deref(), Some("n"));
    }

    #[test]
    fn alias_errors() {
        assert_eq!(msg("SELECT a AS FROM t"), "expected an alias, found FROM");
        assert_eq!(
            msg("SELECT a AS 1 FROM t"),
            "expected an alias, found number `1`"
        );
    }

    #[test]
    fn literals_in_projection() {
        let q = ok("SELECT 1, 2.5, 'x', TRUE, FALSE, NULL, -3, {p:UInt8} FROM t");
        let values: Vec<_> = items(&q)
            .iter()
            .map(|i| match &i.expr {
                Expr::Literal(l) => l.value.clone(),
                e => panic!("expected literal, got {e:?}"),
            })
            .collect();
        assert_eq!(
            values,
            [
                LiteralValue::Numeric("1".into()),
                LiteralValue::Numeric("2.5".into()),
                LiteralValue::String("x".into()),
                LiteralValue::Bool(true),
                LiteralValue::Bool(false),
                LiteralValue::Null,
                LiteralValue::Numeric("-3".into()),
                LiteralValue::Param {
                    name: "p".into(),
                    ty: "UInt8".into()
                },
            ]
        );
    }

    #[test]
    fn negative_literal_span_covers_sign_and_number() {
        let q = ok("SELECT - 42 FROM t");
        assert_eq!(
            items(&q)[0].expr,
            Expr::Literal(lit(LiteralValue::Numeric("-42".into()), 7, 11))
        );
    }

    #[test]
    fn negating_a_column_is_rejected() {
        let e = err("SELECT -a FROM t");
        assert_eq!(
            e.message,
            "expressions are not supported yet; list column names"
        );
        assert_eq!(e.span, Span(7, 9));
        assert_eq!(
            msg("SELECT * FROM t WHERE -a = 1"),
            "arithmetic is not supported yet"
        );
    }

    // ---- function calls ----

    #[test]
    fn calls() {
        let q = ok("SELECT count(*), max(x), now() FROM t");
        assert_eq!(
            items(&q)[0].expr,
            Expr::Call {
                name: "count".into(),
                args: vec![CallArg::Star],
                span: Span(7, 12),
            }
        );
        assert_eq!(
            items(&q)[1].expr,
            Expr::Call {
                name: "max".into(),
                args: vec![CallArg::Expr(Expr::Column(col("x", 21)))],
                span: Span(17, 20),
            }
        );
        assert_eq!(
            items(&q)[2].expr,
            Expr::Call {
                name: "now".into(),
                args: vec![],
                span: Span(25, 28),
            }
        );
    }

    #[test]
    fn call_errors() {
        assert_eq!(
            msg("SELECT f(g(x)) FROM t"),
            "nested functions are not supported yet"
        );
        assert_eq!(
            msg("SELECT f(a, b) FROM t"),
            "functions with more than one argument are not supported yet"
        );
        assert_eq!(
            msg("SELECT f(1) FROM t"),
            "function arguments other than a column are not supported yet"
        );
        assert_eq!(msg("SELECT f(a FROM t"), "expected `)`, found FROM");
        assert_eq!(err("SELECT f(g(x)) FROM t").span, Span(9, 10));
        assert_eq!(err("SELECT f(a, b) FROM t").span, Span(12, 13));
    }

    // ---- projection errors ----

    #[test]
    fn star_cannot_be_mixed_with_columns() {
        assert_eq!(msg("SELECT *, a FROM t"), "cannot mix * with column names");
        assert_eq!(msg("SELECT a, * FROM t"), "cannot mix * with column names");
        assert_eq!(err("SELECT a, * FROM t").span, Span(10, 11));
    }

    #[test]
    fn missing_projection() {
        assert_eq!(
            msg("SELECT FROM t"),
            "expected `*` or a column name, found FROM"
        );
        assert_eq!(
            msg("SELECT a, FROM t"),
            "expected a column name, found FROM"
        );
    }

    #[test]
    fn qualified_columns_are_rejected() {
        let e = err("SELECT t.a FROM t");
        assert_eq!(e.message, "qualified column names are not supported yet");
        assert_eq!(e.span, Span(7, 10));
        assert_eq!(
            msg("SELECT * FROM t WHERE t.a = 1"),
            "qualified column names are not supported yet"
        );
        assert_eq!(
            msg("SELECT * FROM t ORDER BY t.a"),
            "qualified column names are not supported yet"
        );
        assert_eq!(
            msg("SELECT count(t.a) FROM t"),
            "qualified column names are not supported yet"
        );
        assert_eq!(
            msg("SELECT t. FROM t"),
            "expected a column name after `.`, found FROM"
        );
    }

    #[test]
    fn implicit_alias_is_rejected() {
        assert_eq!(
            msg("SELECT a b FROM t"),
            "expected FROM, found identifier `b`"
        );
    }

    // ---- FROM ----

    #[test]
    fn empty_input() {
        let e = err("");
        assert_eq!(e.message, "expected SELECT, found end of input");
        assert_eq!(e.span, Span(0, 0));
    }

    #[test]
    fn missing_from() {
        assert_eq!(msg("SELECT a"), "expected FROM, found end of input");
        assert_eq!(
            msg("SELECT a FROM"),
            "expected a table name, found end of input"
        );
        assert_eq!(
            msg("SELECT a FROM 1"),
            "expected a table name, found number `1`"
        );
    }

    #[test]
    fn final_modifier() {
        let q = ok("SELECT * FROM t FINAL");
        assert_eq!(q.from.final_span, Some(Span(16, 21)));
        assert_eq!(q.from.span, Span(14, 15));
        assert_eq!(
            ok("SELECT * FROM t final").from.final_span,
            Some(Span(16, 21))
        );
    }

    #[test]
    fn table_errors() {
        let e = err("SELECT * FROM db.t");
        assert_eq!(e.message, "qualified table names are not supported");
        assert_eq!(e.span, Span(16, 17));
        assert_eq!(
            msg("SELECT * FROM t AS x"),
            "table aliases are not supported yet"
        );
        assert_eq!(
            msg("SELECT * FROM a, b"),
            "selecting from multiple tables is not supported"
        );
    }

    #[test]
    fn unsupported_clauses() {
        assert_eq!(msg("SELECT * FROM a JOIN b"), "JOIN is not supported yet");
        assert_eq!(
            msg("SELECT * FROM a UNION SELECT * FROM b"),
            "UNION is not supported yet"
        );
        assert_eq!(
            msg("SELECT a FROM t GROUP BY a HAVING a > 1"),
            "HAVING is not supported yet"
        );
        assert_eq!(err("SELECT * FROM a JOIN b").span, Span(16, 20));
    }

    #[test]
    fn trailing_garbage() {
        assert_eq!(
            msg("SELECT * FROM t x"),
            "expected end of query, found identifier `x`"
        );
        assert_eq!(msg("SELECT * FROM t;;"), "expected end of query, found `;`");
        assert_eq!(
            msg("SELECT * FROM t; SELECT * FROM u"),
            "expected end of query, found SELECT"
        );
    }

    // ---- WHERE ----

    #[test]
    fn compare_ops() {
        for (sql_op, op) in [
            ("=", BinOp::Eq),
            ("!=", BinOp::NotEq),
            ("<>", BinOp::NotEq),
            ("<", BinOp::Lt),
            ("<=", BinOp::LtEq),
            (">", BinOp::Gt),
            (">=", BinOp::GtEq),
        ] {
            let src = format!("SELECT * FROM t WHERE a {sql_op} 1");
            let q = ok(&src);
            let rhs = 25 + sql_op.len();
            assert_eq!(
                where_(&q),
                &Expr::Binary {
                    op,
                    left: Box::new(Expr::Column(col("a", 22))),
                    right: Box::new(Expr::Literal(num("1", rhs))),
                    span: Span(24, 24 + sql_op.len()),
                },
                "{src}"
            );
        }
    }

    #[test]
    fn bare_column_as_predicate() {
        let q = ok("SELECT * FROM t WHERE flag");
        assert_eq!(where_(&q), &Expr::Column(col("flag", 22)));
    }

    #[test]
    fn and_binds_tighter_than_or() {
        let q = ok("SELECT * FROM t WHERE a OR b AND c");
        let Expr::Binary {
            op: BinOp::Or,
            left: l,
            right: r,
            ..
        } = where_(&q)
        else {
            panic!("expected OR at the top: {:?}", where_(&q));
        };
        assert_eq!(**l, Expr::Column(col("a", 22)));
        assert!(matches!(**r, Expr::Binary { op: BinOp::And, .. }));
    }

    #[test]
    fn and_or_are_left_associative() {
        let q = ok("SELECT * FROM t WHERE a AND b AND c");
        let Expr::Binary {
            op: BinOp::And,
            left: l,
            right: r,
            ..
        } = where_(&q)
        else {
            panic!("expected AND");
        };
        assert!(matches!(**l, Expr::Binary { op: BinOp::And, .. }));
        assert_eq!(**r, Expr::Column(col("c", 34)));

        let q = ok("SELECT * FROM t WHERE a OR b OR c");
        let Expr::Binary {
            op: BinOp::Or,
            left: l,
            right: r,
            ..
        } = where_(&q)
        else {
            panic!("expected OR");
        };
        assert!(matches!(**l, Expr::Binary { op: BinOp::Or, .. }));
        assert_eq!(**r, Expr::Column(col("c", 32)));
    }

    #[test]
    fn parentheses_override_precedence() {
        let q = ok("SELECT * FROM t WHERE (a OR b) AND c");
        let Expr::Binary {
            op: BinOp::And,
            left: l,
            ..
        } = where_(&q)
        else {
            panic!("expected AND at the top: {:?}", where_(&q));
        };
        assert!(matches!(**l, Expr::Binary { op: BinOp::Or, .. }));
    }

    #[test]
    fn unclosed_paren() {
        assert_eq!(
            msg("SELECT * FROM t WHERE (a = 1"),
            "expected `)`, found end of input"
        );
    }

    #[test]
    fn not_binds_tighter_than_and_and_looser_than_compare() {
        let q = ok("SELECT * FROM t WHERE NOT a = 1 AND b");
        let Expr::Binary {
            op: BinOp::And,
            left: l,
            ..
        } = where_(&q)
        else {
            panic!("expected AND at the top: {:?}", where_(&q));
        };
        let Expr::Unary {
            op: UnaryOp::Not,
            expr,
            span,
        } = &**l
        else {
            panic!("expected NOT: {l:?}");
        };
        assert_eq!(*span, Span(22, 25));
        assert!(matches!(**expr, Expr::Binary { op: BinOp::Eq, .. }));
    }

    #[test]
    fn double_not() {
        let q = ok("SELECT * FROM t WHERE NOT NOT a");
        let Expr::Unary {
            op: UnaryOp::Not,
            expr,
            ..
        } = where_(&q)
        else {
            panic!("expected NOT");
        };
        assert!(matches!(
            **expr,
            Expr::Unary {
                op: UnaryOp::Not,
                ..
            }
        ));
    }

    #[test]
    fn is_null() {
        let q = ok("SELECT * FROM t WHERE a IS NULL");
        assert_eq!(
            where_(&q),
            &Expr::IsNull {
                expr: Box::new(Expr::Column(col("a", 22))),
                negated: false,
                span: Span(24, 26),
            }
        );
        let q = ok("SELECT * FROM t WHERE a IS NOT NULL");
        assert!(matches!(where_(&q), Expr::IsNull { negated: true, .. }));
        assert_eq!(
            msg("SELECT * FROM t WHERE a IS 1"),
            "expected NULL, found number `1`"
        );
        assert_eq!(
            msg("SELECT * FROM t WHERE a IS NOT TRUE"),
            "expected NULL, found TRUE"
        );
    }

    #[test]
    fn in_list() {
        let q = ok("SELECT * FROM t WHERE a IN (1, 'x', -2)");
        assert_eq!(
            where_(&q),
            &Expr::InList {
                expr: Box::new(Expr::Column(col("a", 22))),
                list: vec![
                    Expr::Literal(num("1", 28)),
                    Expr::Literal(lit(LiteralValue::String("x".into()), 31, 34)),
                    Expr::Literal(lit(LiteralValue::Numeric("-2".into()), 36, 38)),
                ],
                negated: false,
                span: Span(24, 26),
            }
        );
    }

    #[test]
    fn not_in_list() {
        let q = ok("SELECT * FROM t WHERE a NOT IN (1)");
        let Expr::InList { negated, span, .. } = where_(&q) else {
            panic!("expected IN list: {:?}", where_(&q));
        };
        assert!(*negated);
        assert_eq!(*span, Span(28, 30));
    }

    #[test]
    fn in_list_errors() {
        assert_eq!(
            msg("SELECT * FROM t WHERE a IN 1"),
            "expected `(`, found number `1`"
        );
        assert_eq!(
            msg("SELECT * FROM t WHERE a IN ()"),
            "expected an expression, found `)`"
        );
        assert_eq!(
            msg("SELECT * FROM t WHERE a IN (1 2)"),
            "expected `,` or `)`, found number `2`"
        );
        assert_eq!(
            msg("SELECT * FROM t WHERE a IN (b)"),
            "only values are supported in IN lists yet"
        );
    }

    #[test]
    fn in_subquery() {
        let q = ok("SELECT * FROM t WHERE a IN (SELECT b FROM u WHERE c = 1)");
        let Expr::InSubquery {
            expr,
            query,
            negated,
            span,
        } = where_(&q)
        else {
            panic!("expected IN sub-query: {:?}", where_(&q));
        };
        assert_eq!(**expr, Expr::Column(col("a", 22)));
        assert!(!negated);
        assert_eq!(*span, Span(24, 26));
        assert_eq!(query.from.name, "u");
        assert!(query.predicate.is_some());

        let q = ok("SELECT * FROM t WHERE a NOT IN (SELECT b FROM u)");
        assert!(matches!(where_(&q), Expr::InSubquery { negated: true, .. }));
    }

    #[test]
    fn in_subquery_errors() {
        assert_eq!(
            msg("SELECT * FROM t WHERE a IN (SELECT b FROM u"),
            "expected `)` after the sub-query, found end of input"
        );
        assert_eq!(
            msg("SELECT * FROM t WHERE a IN (SELECT b FROM u;)"),
            "expected `)` after the sub-query, found `;`"
        );
    }

    #[test]
    fn exists() {
        let q = ok("SELECT * FROM t WHERE EXISTS (SELECT * FROM u)");
        let Expr::Exists {
            query,
            negated,
            span,
        } = where_(&q)
        else {
            panic!("expected EXISTS: {:?}", where_(&q));
        };
        assert!(!negated);
        assert_eq!(*span, Span(22, 28));
        assert_eq!(query.from.name, "u");
    }

    #[test]
    fn not_exists() {
        let q = ok("SELECT * FROM t WHERE NOT EXISTS (SELECT * FROM u)");
        let Expr::Exists { negated, span, .. } = where_(&q) else {
            panic!("expected EXISTS: {:?}", where_(&q));
        };
        assert!(*negated);
        assert_eq!(*span, Span(26, 32));
    }

    #[test]
    fn exists_combined_with_other_predicates() {
        let q = ok("SELECT * FROM t WHERE a = 1 AND EXISTS (SELECT * FROM u) OR b");
        let Expr::Binary {
            op: BinOp::Or,
            left: l,
            ..
        } = where_(&q)
        else {
            panic!("expected OR at the top: {:?}", where_(&q));
        };
        let Expr::Binary {
            op: BinOp::And,
            right: r,
            ..
        } = &**l
        else {
            panic!("expected AND: {l:?}");
        };
        assert!(matches!(**r, Expr::Exists { .. }));
    }

    #[test]
    fn subquery_may_have_all_clauses() {
        let q = ok("SELECT * FROM t WHERE EXISTS \
             (SELECT a FROM u WHERE b = 1 GROUP BY a ORDER BY a LIMIT 1 OFFSET 2)");
        let Expr::Exists { query, .. } = where_(&q) else {
            panic!("expected EXISTS");
        };
        assert!(query.predicate.is_some());
        assert!(query.group_by.is_some());
        assert!(query.order.is_some());
        assert_eq!(query.limit.map(|l| l.0), Some(1));
        assert_eq!(query.offset.map(|o| o.0), Some(2));
    }

    #[test]
    fn nested_subqueries() {
        let q = ok("SELECT * FROM t WHERE EXISTS \
             (SELECT * FROM u WHERE a IN (SELECT b FROM v))");
        let Expr::Exists { query, .. } = where_(&q) else {
            panic!("expected EXISTS");
        };
        assert!(matches!(query.predicate, Some(Expr::InSubquery { .. })));
    }

    #[test]
    fn exists_errors() {
        assert_eq!(
            msg("SELECT * FROM t WHERE EXISTS SELECT * FROM u"),
            "expected `(` and a sub-query after EXISTS, found SELECT"
        );
        assert_eq!(
            msg("SELECT * FROM t WHERE EXISTS (a)"),
            "expected a sub-query (SELECT …) after EXISTS, found identifier `a`"
        );
        assert_eq!(
            msg("SELECT * FROM t WHERE EXISTS (SELECT * FROM u"),
            "expected `)` after the sub-query, found end of input"
        );
    }

    #[test]
    fn scalar_subqueries_are_rejected() {
        let e = err("SELECT * FROM t WHERE a = (SELECT b FROM u)");
        assert_eq!(e.message, "subqueries are not supported");
        assert_eq!(e.span, Span(27, 33));
    }

    #[test]
    fn unsupported_operators() {
        assert_eq!(
            msg("SELECT * FROM t WHERE a LIKE 'x%'"),
            "LIKE is not supported yet"
        );
        assert_eq!(
            msg("SELECT * FROM t WHERE a ILIKE 'x%'"),
            "ILIKE is not supported yet"
        );
        assert_eq!(
            msg("SELECT * FROM t WHERE a BETWEEN 1 AND 2"),
            "BETWEEN is not supported yet"
        );
        assert_eq!(
            msg("SELECT * FROM t WHERE a + 1 = 2"),
            "arithmetic is not supported yet"
        );
        assert_eq!(err("SELECT * FROM t WHERE a + 1 = 2").span, Span(24, 25));
    }

    #[test]
    fn arithmetic_on_the_right_of_a_comparison_is_rejected() {
        assert_eq!(
            msg("SELECT * FROM t WHERE a = b + 1"),
            "arithmetic is not supported yet"
        );
    }

    #[test]
    fn where_with_literals_and_params() {
        let q = ok("SELECT * FROM t WHERE id = {id:UUID} AND name != 'bob' AND x = NULL");
        assert!(matches!(where_(&q), Expr::Binary { op: BinOp::And, .. }));
        let q = ok("SELECT * FROM t WHERE 1 = a");
        let Expr::Binary { left, .. } = where_(&q) else {
            panic!("expected comparison");
        };
        assert_eq!(**left, Expr::Literal(num("1", 22)));
    }

    #[test]
    fn where_call() {
        let q = ok("SELECT * FROM t WHERE length(s) > 3");
        let Expr::Binary { left, .. } = where_(&q) else {
            panic!("expected comparison");
        };
        assert!(matches!(**left, Expr::Call { .. }));
    }

    #[test]
    fn empty_where() {
        assert!(parse_sql("SELECT * FROM t WHERE").is_err());
        assert!(parse_sql("SELECT * FROM t WHERE a =").is_err());
        assert!(parse_sql("SELECT * FROM t WHERE a AND").is_err());
        assert!(parse_sql("SELECT * FROM t WHERE NOT").is_err());
    }

    // ---- GROUP BY ----

    #[test]
    fn group_by() {
        let q = ok("SELECT a, b, count(*) FROM t GROUP BY a, b");
        assert_eq!(
            q.group_by,
            Some((vec![col("a", 38), col("b", 41)], Span(29, 34)))
        );
    }

    #[test]
    fn group_by_errors() {
        assert_eq!(
            msg("SELECT a FROM t GROUP a"),
            "expected BY, found identifier `a`"
        );
        assert_eq!(
            msg("SELECT a FROM t GROUP BY"),
            "expected a column name, found end of input"
        );
        assert_eq!(
            msg("SELECT a FROM t GROUP BY ;"),
            "expected a column name, found `;`"
        );
        assert_eq!(
            msg("SELECT a FROM t GROUP BY a,"),
            "expected a column name, found end of input"
        );
        let e = err("SELECT a FROM t GROUP BY 1");
        assert_eq!(
            e.message,
            "grouping by position is not supported yet; name the column"
        );
        assert_eq!(e.span, Span(25, 26));
        assert_eq!(
            msg("SELECT a FROM t GROUP BY a + 1"),
            "arithmetic is not supported yet"
        );
        assert_eq!(
            msg("SELECT a FROM t GROUP BY f(a)"),
            "grouping by expressions is not supported yet; name a column"
        );
    }

    #[test]
    fn group_by_all() {
        let e = err("SELECT a FROM t GROUP BY ALL");
        assert_eq!(
            e.message,
            "GROUP BY ALL is not supported yet; name the columns"
        );
        assert_eq!(e.span, Span(25, 28));
    }

    // ---- ORDER BY ----

    #[test]
    fn order_by() {
        let q = ok("SELECT * FROM t ORDER BY a, b DESC, c ASC NULLS FIRST, d NULLS LAST");
        assert_eq!(
            order_items(&q),
            [
                OrderItem {
                    expr: Expr::Column(col("a", 25)),
                    direction: None,
                    nulls: None,
                },
                OrderItem {
                    expr: Expr::Column(col("b", 28)),
                    direction: Some(Direction::Desc),
                    nulls: None,
                },
                OrderItem {
                    expr: Expr::Column(col("c", 36)),
                    direction: Some(Direction::Asc),
                    nulls: Some(NullsOrder::First),
                },
                OrderItem {
                    expr: Expr::Column(col("d", 55)),
                    direction: None,
                    nulls: Some(NullsOrder::Last),
                },
            ]
        );
    }

    #[test]
    fn order_modifiers_are_case_insensitive() {
        let q = ok("SELECT * FROM t ORDER BY a desc nulls last");
        assert_eq!(order_items(&q)[0].direction, Some(Direction::Desc));
        assert_eq!(order_items(&q)[0].nulls, Some(NullsOrder::Last));
    }

    #[test]
    fn order_by_unreserved_word_columns() {
        let q = ok("SELECT * FROM t ORDER BY desc ASC, asc DESC");
        let items = order_items(&q);
        assert_eq!(items[0].expr, Expr::Column(col("desc", 25)));
        assert_eq!(items[0].direction, Some(Direction::Asc));
        assert_eq!(items[1].expr, Expr::Column(col("asc", 35)));
        assert_eq!(items[1].direction, Some(Direction::Desc));
    }

    #[test]
    fn order_by_call() {
        let q = ok("SELECT * FROM t ORDER BY lower(name) DESC");
        assert!(matches!(order_items(&q)[0].expr, Expr::Call { .. }));
        assert_eq!(order_items(&q)[0].direction, Some(Direction::Desc));
    }

    #[test]
    fn order_by_all() {
        let q = ok("SELECT * FROM t ORDER BY ALL");
        assert_eq!(
            q.order,
            Some(Order::All {
                direction: None,
                nulls: None,
                span: Span(25, 28),
            })
        );
        let q = ok("SELECT * FROM t ORDER BY ALL DESC NULLS FIRST");
        assert_eq!(
            q.order,
            Some(Order::All {
                direction: Some(Direction::Desc),
                nulls: Some(NullsOrder::First),
                span: Span(25, 28),
            })
        );
        assert!(matches!(
            ok("SELECT * FROM t ORDER BY all desc").order,
            Some(Order::All {
                direction: Some(Direction::Desc),
                ..
            })
        ));
        assert_eq!(
            msg("SELECT * FROM t ORDER BY ALL, a"),
            "ORDER BY ALL can't be combined with other columns"
        );
    }

    #[test]
    fn order_by_errors() {
        assert_eq!(
            msg("SELECT * FROM t ORDER a"),
            "expected BY, found identifier `a`"
        );
        assert_eq!(
            msg("SELECT * FROM t ORDER BY"),
            "expected a column name, found end of input"
        );
        assert_eq!(
            msg("SELECT * FROM t ORDER BY a,"),
            "expected a column name, found end of input"
        );
        let e = err("SELECT * FROM t ORDER BY 1");
        assert_eq!(
            e.message,
            "ordering by position is not supported yet; name the column"
        );
        assert_eq!(e.span, Span(25, 26));
        assert_eq!(
            msg("SELECT * FROM t ORDER BY a + 1"),
            "ordering by expressions is not supported yet; name a column"
        );
        assert_eq!(
            msg("SELECT * FROM t ORDER BY a = 1"),
            "ordering by expressions is not supported yet; name a column"
        );
        assert_eq!(
            msg("SELECT * FROM t ORDER BY 'a'"),
            "ordering by expressions is not supported yet; name a column"
        );
        assert_eq!(
            msg("SELECT * FROM t ORDER BY a NULLS"),
            "expected FIRST or LAST, found end of input"
        );
        assert_eq!(
            msg("SELECT * FROM t ORDER BY a NULLS DESC"),
            "expected FIRST or LAST, found identifier `DESC`"
        );
        assert_eq!(
            msg("SELECT * FROM t ORDER BY a COLLATE 'de'"),
            "COLLATE in ORDER BY is not supported"
        );
        assert_eq!(
            msg("SELECT * FROM t ORDER BY a DESC WITH FILL"),
            "WITH in ORDER BY is not supported"
        );
    }

    #[test]
    fn order_by_must_come_after_group_by() {
        assert!(parse_sql("SELECT a FROM t ORDER BY a GROUP BY a").is_err());
    }

    // ---- LIMIT / OFFSET ----

    #[test]
    fn limit() {
        let q = ok("SELECT * FROM t LIMIT 10");
        assert_eq!(q.limit, Some((10, Span(22, 24))));
        assert_eq!(q.offset, None);
    }

    #[test]
    fn limit_offset() {
        let q = ok("SELECT * FROM t LIMIT 10 OFFSET 5");
        assert_eq!(q.limit, Some((10, Span(22, 24))));
        assert_eq!(q.offset, Some((5, Span(32, 33))));
    }

    #[test]
    fn limit_comma_form_is_offset_then_limit() {
        let q = ok("SELECT * FROM t LIMIT 5, 10");
        assert_eq!(q.offset, Some((5, Span(22, 23))));
        assert_eq!(q.limit, Some((10, Span(25, 27))));
    }

    #[test]
    fn offset_without_limit() {
        let q = ok("SELECT * FROM t offset 3");
        assert_eq!(q.limit, None);
        assert_eq!(q.offset, Some((3, Span(23, 24))));
    }

    #[test]
    fn offset_after_comma_limit_is_rejected() {
        assert!(parse_sql("SELECT * FROM t LIMIT 5, 10 OFFSET 1").is_err());
    }

    #[test]
    fn limit_errors() {
        assert_eq!(
            msg("SELECT * FROM t LIMIT"),
            "LIMIT needs a non-negative whole number"
        );
        assert_eq!(
            msg("SELECT * FROM t LIMIT -1"),
            "LIMIT needs a non-negative whole number"
        );
        assert_eq!(
            msg("SELECT * FROM t LIMIT 1.5"),
            "LIMIT needs a non-negative whole number"
        );
        assert_eq!(
            msg("SELECT * FROM t LIMIT a"),
            "LIMIT needs a non-negative whole number"
        );
        assert_eq!(
            msg("SELECT * FROM t LIMIT 99999999999999999999"),
            "LIMIT needs a non-negative whole number"
        );
        assert_eq!(
            msg("SELECT * FROM t LIMIT 5,"),
            "LIMIT needs a non-negative whole number"
        );
        assert_eq!(
            msg("SELECT * FROM t OFFSET -1"),
            "OFFSET needs a non-negative whole number"
        );
        assert_eq!(err("SELECT * FROM t LIMIT 1.5").span, Span(22, 25));
        assert_eq!(
            msg("SELECT * FROM t LIMIT 1 WITH TIES"),
            "LIMIT … WITH TIES is not supported yet"
        );
        assert_eq!(
            msg("SELECT * FROM t LIMIT 1 BY a"),
            "LIMIT … BY is not supported yet"
        );
        assert_eq!(
            msg("SELECT * FROM t LIMIT 1, 2 BY a"),
            "LIMIT … BY is not supported yet"
        );
    }

    #[test]
    fn limit_zero() {
        assert_eq!(ok("SELECT * FROM t LIMIT 0").limit.map(|l| l.0), Some(0));
    }

    // ---- everything together ----

    #[test]
    fn full_query() {
        let q = ok("SELECT a, count(*) AS n FROM events FINAL \
             WHERE (a = 1 OR b IS NOT NULL) AND c NOT IN ('x', 'y') \
             GROUP BY a \
             ORDER BY n DESC NULLS LAST \
             LIMIT 10 OFFSET 20;");
        assert_eq!(items(&q).len(), 2);
        assert_eq!(q.from.name, "events");
        assert!(q.from.final_span.is_some());
        let Expr::Binary {
            op: BinOp::And,
            left: l,
            right: r,
            ..
        } = where_(&q)
        else {
            panic!("expected AND at the top");
        };
        assert!(matches!(**l, Expr::Binary { op: BinOp::Or, .. }));
        assert!(matches!(**r, Expr::InList { negated: true, .. }));
        assert_eq!(q.group_by.as_ref().map(|g| g.0.len()), Some(1));
        assert_eq!(order_items(&q)[0].direction, Some(Direction::Desc));
        assert_eq!(q.limit.map(|l| l.0), Some(10));
        assert_eq!(q.offset.map(|o| o.0), Some(20));
    }

    #[test]
    fn clauses_out_of_order_are_rejected() {
        assert!(parse_sql("SELECT * FROM t LIMIT 1 WHERE a").is_err());
        assert!(parse_sql("SELECT * FROM t ORDER BY a WHERE a").is_err());
        assert!(parse_sql("SELECT * FROM t LIMIT 1 ORDER BY a").is_err());
    }

    #[test]
    fn comments_and_whitespace_are_ignored() {
        assert_eq!(
            ok("SELECT * -- everything\nFROM t\n\t;").projection,
            Projection::Wildcard
        );
    }

    // ---- general expressions ----

    #[test]
    fn parentheses_around_keys_are_transparent() {
        assert_eq!(
            ok("SELECT a FROM t GROUP BY (a)").group_by,
            Some((vec![col("a", 26)], Span(16, 21)))
        );
        assert_eq!(
            order_items(&ok("SELECT * FROM t ORDER BY ((a)) DESC"))[0].expr,
            Expr::Column(col("a", 27))
        );
    }

    #[test]
    fn product_binds_tighter_than_sum() {
        // Arithmetic is parsed before it's rejected, so the error points at the operator
        // that ends up at the top of the tree.
        let e = err("SELECT * FROM t WHERE a + b * c = 1");
        assert_eq!(e.message, "arithmetic is not supported yet");
        assert_eq!(e.span, Span(24, 25));
        let e = err("SELECT * FROM t WHERE a * b + c = 1");
        assert_eq!(e.span, Span(28, 29));
    }

    #[test]
    fn arithmetic_after_a_value_is_rejected() {
        let e = err("SELECT * FROM t WHERE a = 1 - 2");
        assert_eq!(e.message, "arithmetic is not supported yet");
        assert_eq!(e.span, Span(28, 29));
    }

    #[test]
    fn comparisons_bind_tighter_than_not() {
        let q = ok("SELECT * FROM t WHERE NOT a = 1 AND b IS NULL");
        let Expr::Binary {
            op: BinOp::And,
            left,
            right,
            ..
        } = where_(&q)
        else {
            panic!("expected AND at the top: {:?}", where_(&q));
        };
        assert!(matches!(
            &**left,
            Expr::Unary { op: UnaryOp::Not, expr, .. }
                if matches!(**expr, Expr::Binary { op: BinOp::Eq, .. })
        ));
        assert!(matches!(**right, Expr::IsNull { negated: false, .. }));
    }

    #[test]
    fn between_and_does_not_split_the_condition() {
        let e = err("SELECT * FROM t WHERE x = 1 AND a BETWEEN 1 AND 2 AND y = 2");
        assert_eq!(e.message, "BETWEEN is not supported yet");
        assert_eq!(e.span, Span(34, 41));
        assert_eq!(
            msg("SELECT * FROM t WHERE a NOT BETWEEN 1 AND 2"),
            "BETWEEN is not supported yet"
        );
        assert_eq!(
            msg("SELECT * FROM t WHERE a BETWEEN 1"),
            "expected AND, found end of input"
        );
    }

    #[test]
    fn not_like() {
        let e = err("SELECT * FROM t WHERE a NOT LIKE 'x%'");
        assert_eq!(e.message, "LIKE is not supported yet");
        assert_eq!(e.span, Span(28, 32));
        assert_eq!(
            msg("SELECT * FROM t WHERE a NOT ILIKE 'x%'"),
            "ILIKE is not supported yet"
        );
    }

    #[test]
    fn conditions_are_rejected_in_the_projection() {
        let e = err("SELECT a = 1 FROM t");
        assert_eq!(
            e.message,
            "expressions are not supported yet; list column names"
        );
        assert_eq!(e.span, Span(7, 12));
        assert_eq!(
            msg("SELECT a + 1 FROM t"),
            "expressions are not supported yet; list column names"
        );
    }

    #[test]
    fn empty_where_expects_an_expression() {
        assert_eq!(
            msg("SELECT * FROM t WHERE"),
            "expected an expression, found end of input"
        );
        assert_eq!(
            msg("SELECT * FROM t WHERE a = ORDER BY a"),
            "expected an expression, found ORDER"
        );
    }

    #[test]
    fn expr_span_covers_the_whole_expression() {
        // Closing parentheses aren't in the tree, so the extent ends at the last element.
        let q = ok("SELECT * FROM t WHERE a = 1 AND b IN (1, 2)");
        assert_eq!(where_(&q).span(), Span(22, 42));
        let Expr::Binary { span, .. } = where_(&q) else {
            panic!("expected AND");
        };
        assert_eq!(*span, Span(28, 31));
    }
}
