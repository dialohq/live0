use std::sync::Arc;

use catalog::{Catalog, ColumnId, Table};
use io::ColumnDefault;
use parser::{
    Query,
    ast::{Expr as AstExpr, Order, Projection, SelectItem, TableRef},
    error::Span,
};
use types::DataType;

use crate::{Access, Error, Grant, Params};

/// The query with names resolved, types checked, parameters bound, security applied and normalized.
pub(crate) struct BoundQuery {
    pub table: Arc<Table>,
    /// Result columns, in order.
    pub columns: Vec<BoundColumn>,
}

pub(crate) struct BoundColumn {
    /// The alias, or the column's name.
    pub name: String,
    pub column: ColumnId,
    pub ty: DataType,
}

pub(crate) fn bind(
    query: &Query,
    catalog: &Catalog,
    _params: &Params,
    access: &Access,
) -> Result<BoundQuery, Error> {
    unsupported(query)?;
    let table = bind_from(&query.from, catalog, access)?;
    let columns = bind_projection(&query.projection, &table, access)?;
    Ok(BoundQuery { table, columns })
}

fn error(message: impl Into<String>, span: Span) -> Error {
    Error {
        message: message.into(),
        span: Some(span),
    }
}

/// Clauses that aren't bound yet; ignoring one would give a wrong result.
fn unsupported(query: &Query) -> Result<(), Error> {
    let not_yet =
        |clause: &str, span: Span| Err(error(format!("{clause} isn't supported yet"), span));
    if let Some(span) = query.from.final_span {
        return not_yet("FINAL", span);
    }
    if let Some(predicate) = &query.predicate {
        return not_yet("WHERE", predicate.span());
    }
    if let Some((_, span)) = &query.group_by {
        return not_yet("GROUP BY", *span);
    }
    match &query.order {
        Some(Order::All { span, .. }) => return not_yet("ORDER BY", *span),
        Some(Order::Items(items)) => {
            if let Some(item) = items.first() {
                return not_yet("ORDER BY", item.expr.span());
            }
        }
        None => {}
    }
    if let Some((_, span)) = query.limit {
        return not_yet("LIMIT", span);
    }
    if let Some((_, span)) = query.offset {
        return not_yet("OFFSET", span);
    }
    Ok(())
}

fn granted(access: &Access, grant: &str) -> bool {
    match access {
        Access::Unrestricted => true,
        Access::Grants(grants) => matches!(grants.get(grant), Some(Grant::Check(true))),
    }
}

fn bind_from(from: &TableRef, catalog: &Catalog, access: &Access) -> Result<Arc<Table>, Error> {
    let table = catalog
        .table(&from.name)
        .ok_or_else(|| error(format!("no table `{}`", from.name), from.span))?;
    if let Some(grant) = &table.security.grant
        && !granted(access, grant)
    {
        return Err(error(
            format!("no access to table `{}`", from.name),
            from.span,
        ));
    }
    // Row rules need a filter, which comes with WHERE; ignoring them would show rows the subject mustn't see.
    if matches!(access, Access::Grants(_)) && !table.security.rows.is_empty() {
        return Err(error("row security isn't supported yet", from.span));
    }
    Ok(table.clone())
}

fn bind_projection(
    projection: &Projection,
    table: &Table,
    access: &Access,
) -> Result<Vec<BoundColumn>, Error> {
    let visible = |id: ColumnId| {
        table
            .security
            .columns
            .iter()
            .filter(|rule| rule.column == id)
            .all(|rule| granted(access, &rule.grant))
    };
    match projection {
        Projection::Wildcard => table
            .columns
            .iter()
            .enumerate()
            .filter(|(id, c)| {
                visible(*id) && matches!(c.default, ColumnDefault::None | ColumnDefault::Default)
            })
            .map(|(id, c)| match &c.ty {
                DataType::Other(ty) => Err(Error {
                    message: format!(
                        "column `{}` has unsupported type `{ty}`; list the columns to read",
                        c.name
                    ),
                    span: None,
                }),
                ty => Ok(BoundColumn {
                    name: c.name.clone(),
                    column: id,
                    ty: ty.clone(),
                }),
            })
            .collect(),
        Projection::Items(items) => items
            .iter()
            .map(|item| bind_item(item, table, &visible))
            .collect(),
    }
}

fn bind_item(
    item: &SelectItem,
    table: &Table,
    visible: &impl Fn(ColumnId) -> bool,
) -> Result<BoundColumn, Error> {
    let AstExpr::Column(column) = &item.expr else {
        return Err(error(
            "expressions aren't supported yet; list column names",
            item.expr.span(),
        ));
    };
    if let Some(qualifier) = &column.qualifier
        && *qualifier != table.name
    {
        return Err(error(format!("no table `{qualifier}`"), column.span));
    }
    let (id, found) = table
        .column(&column.name)
        .filter(|(id, _)| visible(*id))
        .ok_or_else(|| error(format!("no column `{}`", column.name), column.span))?;
    let reason = match (&found.default, &found.ty) {
        (ColumnDefault::Materialized | ColumnDefault::Alias, _) => {
            Some("is computed by ClickHouse and can't be read live".to_owned())
        }
        (ColumnDefault::Ephemeral, _) => Some("is EPHEMERAL and can't be read".to_owned()),
        (_, DataType::Other(ty)) => Some(format!("has unsupported type `{ty}`")),
        _ => None,
    };
    if let Some(reason) = reason {
        return Err(error(
            format!("column `{}` {reason}", found.name),
            column.span,
        ));
    }
    Ok(BoundColumn {
        name: item.alias.clone().unwrap_or_else(|| found.name.clone()),
        column: id,
        ty: found.ty.clone(),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::testing::{catalog, grants};

    async fn bind_sql(sql: &str, access: &Access) -> Result<BoundQuery, Error> {
        bind(
            &parser::parse(sql).unwrap(),
            &catalog().await,
            &Params::new(),
            access,
        )
    }

    async fn columns(sql: &str, access: &Access) -> Vec<(String, ColumnId)> {
        match bind_sql(sql, access).await {
            Ok(bound) => bound
                .columns
                .into_iter()
                .map(|c| (c.name, c.column))
                .collect(),
            Err(err) => panic!("{sql}: {err}"),
        }
    }

    /// The error's message and the source text its span covers.
    async fn failure(sql: &str, access: &Access) -> (String, Option<String>) {
        match bind_sql(sql, access).await {
            Ok(_) => panic!("{sql} bound"),
            Err(err) => (err.message, err.span.map(|Span(a, b)| sql[a..b].to_owned())),
        }
    }

    fn named(pairs: &[(&str, ColumnId)]) -> Vec<(String, ColumnId)> {
        pairs.iter().map(|(n, id)| (n.to_string(), *id)).collect()
    }

    #[tokio::test]
    async fn wildcard_expands_readable_columns() {
        let bound = bind_sql("SELECT * FROM events", &Access::Unrestricted)
            .await
            .unwrap();
        assert_eq!(bound.table.name, "events");
        let got: Vec<_> = bound
            .columns
            .iter()
            .map(|c| (c.name.as_str(), c.column, c.ty.clone()))
            .collect();
        let string = DataType::parse("String");
        assert_eq!(
            got,
            [
                ("id", 0, DataType::parse("UInt64")),
                ("name", 1, string.clone()),
                ("email", 2, string),
            ]
        );
    }

    #[tokio::test]
    async fn wildcard_with_unsupported_column_errors() {
        let (message, _) = failure("SELECT * FROM geo", &Access::Unrestricted).await;
        assert!(
            message.contains("`spot` has unsupported type `Point`"),
            "{message}"
        );
        assert_eq!(
            columns("SELECT id FROM geo", &Access::Unrestricted).await,
            named(&[("id", 0)])
        );
    }

    #[tokio::test]
    async fn named_columns_keep_order_and_aliases() {
        assert_eq!(
            columns("SELECT name AS n, id FROM events", &Access::Unrestricted).await,
            named(&[("n", 1), ("id", 0)])
        );
    }

    /// The parser doesn't produce qualified names yet, so the qualifier is set on a parsed query.
    #[tokio::test]
    async fn qualified_columns() {
        let qualified = |qualifier: &str| {
            let mut query = parser::parse("SELECT id FROM events").unwrap();
            if let Projection::Items(items) = &mut query.projection
                && let AstExpr::Column(column) = &mut items[0].expr
            {
                column.qualifier = Some(qualifier.to_owned());
            }
            query
        };
        let catalog = catalog().await;
        let bind = |query| bind(&query, &catalog, &Params::new(), &Access::Unrestricted);
        assert_eq!(bind(qualified("events")).unwrap().columns[0].column, 0);
        let err = bind(qualified("other")).err().unwrap();
        assert_eq!(
            (err.message.as_str(), err.span),
            ("no table `other`", Some(Span(7, 9)))
        );
    }

    #[tokio::test]
    async fn unknown_column_has_span() {
        let (message, span) = failure("SELECT id, nope FROM events", &Access::Unrestricted).await;
        assert_eq!(
            (message.as_str(), span.as_deref()),
            ("no column `nope`", Some("nope"))
        );
    }

    #[tokio::test]
    async fn computed_and_ephemeral_columns_are_rejected() {
        for (column, reason) in [
            ("upper", "computed by ClickHouse"),
            ("alias_name", "computed by ClickHouse"),
            ("eph", "EPHEMERAL"),
        ] {
            let sql = format!("SELECT {column} FROM events");
            let (message, span) = failure(&sql, &Access::Unrestricted).await;
            assert!(message.contains(reason), "{message}");
            assert_eq!(span.as_deref(), Some(column));
        }
    }

    #[tokio::test]
    async fn unknown_table_has_span() {
        let (message, span) = failure("SELECT * FROM nope", &Access::Unrestricted).await;
        assert_eq!(
            (message.as_str(), span.as_deref()),
            ("no table `nope`", Some("nope"))
        );
    }

    #[tokio::test]
    async fn table_grant() {
        for access in [
            grants(&[]),
            Access::Grants(BTreeMap::from([(
                "events:read".to_owned(),
                Grant::Check(false),
            )])),
        ] {
            let (message, _) = failure("SELECT id FROM events", &access).await;
            assert_eq!(message, "no access to table `events`");
        }
        assert_eq!(
            columns("SELECT id FROM events", &grants(&["events:read"])).await,
            named(&[("id", 0)])
        );
    }

    #[tokio::test]
    async fn hidden_columns() {
        let reader = grants(&["events:read"]);
        assert_eq!(
            columns("SELECT * FROM events", &reader).await,
            named(&[("id", 0), ("name", 1)])
        );
        let (message, _) = failure("SELECT email FROM events", &reader).await;
        assert_eq!(message, "no column `email`");
        assert_eq!(
            columns("SELECT * FROM events", &grants(&["events:read", "pii"])).await,
            named(&[("id", 0), ("name", 1), ("email", 2)])
        );
    }

    #[tokio::test]
    async fn row_rules_are_rejected_for_now() {
        let (message, _) = failure("SELECT * FROM tenants", &grants(&[])).await;
        assert_eq!(message, "row security isn't supported yet");
        assert_eq!(
            columns("SELECT * FROM tenants", &Access::Unrestricted).await,
            named(&[("id", 0), ("tenant", 1)])
        );
    }

    #[tokio::test]
    async fn unsupported_clauses_have_spans() {
        for (sql, clause, at) in [
            ("SELECT * FROM events WHERE id = 1", "WHERE", "id = 1"),
            ("SELECT id FROM events GROUP BY id", "GROUP BY", "GROUP"),
            ("SELECT * FROM events ORDER BY id", "ORDER BY", "id"),
            ("SELECT * FROM events LIMIT 5", "LIMIT", "5"),
            ("SELECT * FROM events OFFSET 2", "OFFSET", "2"),
            ("SELECT * FROM events FINAL", "FINAL", "FINAL"),
        ] {
            let (message, span) = failure(sql, &Access::Unrestricted).await;
            assert_eq!(message, format!("{clause} isn't supported yet"), "{sql}");
            assert!(
                span.as_deref().is_some_and(|s| s.starts_with(at)),
                "{sql}: {span:?}"
            );
        }
    }

    #[test]
    fn error_format_points_at_the_span() {
        let sql = "SELECT nope FROM events";
        let with = Error {
            message: "no column `nope`".to_owned(),
            span: Some(Span(7, 11)),
        };
        let formatted = with.format(sql);
        assert!(formatted.contains("no column `nope`"), "{formatted}");
        assert!(formatted.contains("^^^^"), "{formatted}");
        let without = Error {
            message: "plain".to_owned(),
            span: None,
        };
        assert_eq!(without.format(sql), "plain");
    }
}
