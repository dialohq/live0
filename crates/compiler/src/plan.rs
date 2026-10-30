use std::{collections::BTreeMap, fmt, sync::Arc, time::Duration};

use types::{Column, Schema};

use crate::{CompareOp, Error, Expr, HistoryQueries, HistorySql, bind::BoundQuery};

/// A source's position in the plan's `sources`; the outer query reads source 0.
pub type SourceId = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fingerprint(pub [u8; 16]);

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|b| write!(f, "{b:02x}"))
    }
}

/// A table the plan reads, and the fields read from it, in row order.
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    pub table: String,
    pub topic: String,
    pub schema: Schema,
}

#[derive(Debug, Clone)]
pub struct Plan {
    fingerprint: Fingerprint,
    schema: Schema,
    sources: Vec<Source>,
    filter: Option<Expr>,
    projection: Vec<Expr>,
    shape: Shape,
    history: HistoryQueries,
}

/// How a query's state is kept; `qstate` builds one private implementation per variant.
#[derive(Debug, Clone)]
pub enum Shape {
    /// Unordered, unlimited.
    Append,
    /// `LIMIT` without `ORDER BY`: the latest `limit` rows.
    Window { limit: u64 },
    Ordered {
        order: Vec<SortKey>,
        limit: Option<u64>,
    },
    Offset {
        order: Vec<SortKey>,
        offset: u64,
        limit: Option<u64>,
    },
    /// `FINAL` on a ReplacingMergeTree: every event is a new version of its key's row.
    Keyed {
        key: Vec<usize>,
        version: Option<usize>,
        is_deleted: Option<usize>,
        order: Vec<SortKey>,
        limit: Option<u64>,
    },
    Aggregate {
        group: Vec<usize>,
        aggregates: Vec<Aggregate>,
        order: Vec<SortKey>,
        limit: Option<u64>,
    },
    /// Rows enter and leave as time passes; wraps the shape of the rest of the query.
    Timeline {
        bounds: Vec<TimeBound>,
        inner: Box<Shape>,
    },
    /// Outer rows whose key isn't in the inner set yet wait in a tail.
    SemiJoin {
        outer_key: Expr,
        inner: SourceId,
        inner_key: Expr,
        inner_filter: Option<Expr>,
        outer: Box<Shape>,
    },
}

/// One `ORDER BY` item, evaluated on the source row.
#[derive(Debug, Clone, PartialEq)]
pub struct SortKey {
    pub expr: Expr,
    pub descending: bool,
    pub nulls_first: bool,
}

#[derive(Debug, Clone)]
pub struct Aggregate {
    pub function: Arc<dyn functions::Aggregate>,
    pub args: Vec<Expr>,
}

/// `expr op now() + offset`: the row is in the result only while it holds.
#[derive(Debug, Clone, PartialEq)]
pub struct TimeBound {
    pub expr: Expr,
    pub op: CompareOp,
    pub offset: Duration,
}

impl Plan {
    /// Equal for queries that would produce the same result, so it identifies cursors and shared results.
    pub fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }

    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// One per table, with the union of the fields any part of the query reads from it.
    pub fn sources(&self) -> &[Source] {
        &self.sources
    }

    /// The time-independent part of the outer query's `WHERE`, on source 0's rows.
    pub fn filter(&self) -> Option<&Expr> {
        self.filter.as_ref()
    }

    /// One expression per result column, on source 0's rows.
    pub fn projection(&self) -> &[Expr] {
        &self.projection
    }

    pub fn shape(&self) -> &Shape {
        &self.shape
    }

    pub fn history(&self) -> &HistoryQueries {
        &self.history
    }
}

/// A human-readable description of the plan, for `EXPLAIN` and debug pages.
impl fmt::Display for Plan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{} {}", self.shape.name(), self.fingerprint)?;
        for (id, source) in self.sources.iter().enumerate() {
            let fields: Vec<&str> = source
                .schema
                .columns
                .iter()
                .map(|c| c.name.as_str())
                .collect();
            writeln!(
                f,
                "  read {id}: {} (topic {}): {}",
                source.table,
                source.topic,
                fields.join(", ")
            )?;
        }
        if let Some(filter) = &self.filter {
            writeln!(f, "  filter: {filter:?}")?;
        }
        let output: Vec<String> = self
            .schema
            .columns
            .iter()
            .zip(&self.projection)
            .map(|(column, expr)| match expr {
                Expr::Field(i) => {
                    let field = self.sources[0]
                        .schema
                        .columns
                        .get(*i)
                        .map_or("?", |c| c.name.as_str());
                    format!("{} = $0.{field}", column.name)
                }
                expr => format!("{} = {expr:?}", column.name),
            })
            .collect();
        writeln!(f, "  output: {}", output.join(", "))?;
        let history = [
            ("full", Some(&self.history.full)),
            ("seed", self.history.seed.as_ref()),
            ("refill", self.history.refill.as_ref()),
            ("by_key", self.history.by_key.as_ref()),
        ];
        for (name, sql) in history {
            if let Some(sql) = sql {
                writeln!(f, "  history {name}: {}", sql.sql())?;
            }
        }
        Ok(())
    }
}

impl Shape {
    fn name(&self) -> &'static str {
        match self {
            Shape::Append => "append",
            Shape::Window { .. } => "window",
            Shape::Ordered { .. } => "ordered",
            Shape::Offset { .. } => "offset",
            Shape::Keyed { .. } => "keyed",
            Shape::Aggregate { .. } => "aggregate",
            Shape::Timeline { .. } => "timeline",
            Shape::SemiJoin { .. } => "semi-join",
        }
    }
}

/// The append shape: source 0 holds the columns the projection reads, in table order.
pub(crate) fn plan(query: BoundQuery) -> Result<Plan, Error> {
    let table = &query.table;
    let mut read: Vec<_> = query.columns.iter().map(|c| c.column).collect();
    read.sort_unstable();
    read.dedup();
    let position: BTreeMap<_, _> = read.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let source = Source {
        table: table.name.clone(),
        topic: table.topic.clone(),
        schema: Schema {
            columns: read
                .iter()
                .map(|&id| {
                    Column::new(table.columns[id].name.clone(), table.columns[id].ty.clone())
                })
                .collect(),
        },
    };
    let projection = query
        .columns
        .iter()
        .map(|c| Expr::Field(position[&c.column]))
        .collect();
    let schema = Schema {
        columns: query
            .columns
            .iter()
            .map(|c| Column::new(c.name.clone(), c.ty.clone()))
            .collect(),
    };
    let selects: Vec<String> = source
        .schema
        .columns
        .iter()
        .map(|c| c.ty.history_expr(&io::quote_identifier(&c.name)))
        .collect();
    let full = HistorySql::new(
        format!("SELECT {} FROM {{t0:Identifier}}", selects.join(", ")),
        Vec::new(),
        BTreeMap::from([(table.name.clone(), "t0".to_owned())]),
        Vec::new(),
        source.schema.clone(),
    );
    let columns: Vec<(&str, usize)> = query
        .columns
        .iter()
        .map(|c| (c.name.as_str(), c.column))
        .collect();
    Ok(Plan {
        fingerprint: fingerprint(&(&table.name, table.schema_hash, "append", columns)),
        schema,
        sources: vec![source],
        filter: None,
        projection,
        shape: Shape::Append,
        history: HistoryQueries {
            full,
            seed: None,
            refill: None,
            by_key: None,
        },
    })
}

/// FNV-1a 128 of the `Debug` text, so it's the same in every process that reads a cursor.
fn fingerprint(parts: &impl fmt::Debug) -> Fingerprint {
    let hash = format!("{parts:?}").bytes().fold(
        0x6c62_272e_07bb_0142_62b8_2175_6295_c58d_u128,
        |hash, byte| {
            (hash ^ u128::from(byte)).wrapping_mul(0x0000_0000_0100_0000_0000_0000_0000_013b)
        },
    );
    Fingerprint(hash.to_be_bytes())
}

#[cfg(test)]
mod tests {
    use types::{DataType, Value};

    use super::*;
    use crate::{Access, Params, compile, testing::catalog};

    async fn plan_sql(sql: &str) -> Plan {
        let catalog = catalog().await;
        match compile(
            &parser::parse(sql).unwrap(),
            &catalog,
            &Params::new(),
            &Access::Unrestricted,
        ) {
            Ok(plan) => plan,
            Err(err) => panic!("{sql}: {err}"),
        }
    }

    fn names(schema: &Schema) -> Vec<&str> {
        schema.columns.iter().map(|c| c.name.as_str()).collect()
    }

    fn fields(plan: &Plan) -> Vec<Expr> {
        plan.projection().to_vec()
    }

    #[tokio::test]
    async fn select_star_is_one_append_source() {
        let plan = plan_sql("SELECT * FROM events").await;
        assert!(matches!(plan.shape(), Shape::Append));
        assert_eq!(plan.sources().len(), 1);
        let source = &plan.sources()[0];
        assert_eq!(
            (source.table.as_str(), source.topic.as_str()),
            ("events", "events")
        );
        assert_eq!(names(&source.schema), ["id", "name", "email"]);
        assert_eq!(source.schema.columns[0].ty, DataType::parse("UInt64"));
        assert_eq!(
            fields(&plan),
            [Expr::Field(0), Expr::Field(1), Expr::Field(2)]
        );
        assert_eq!(plan.schema(), &source.schema);
        assert!(plan.filter().is_none());
    }

    #[tokio::test]
    async fn projection_reads_each_column_once_in_table_order() {
        let plan = plan_sql("SELECT name AS n, id, name FROM events").await;
        assert_eq!(names(&plan.sources()[0].schema), ["id", "name"]);
        assert_eq!(
            fields(&plan),
            [Expr::Field(1), Expr::Field(0), Expr::Field(1)]
        );
        assert_eq!(names(plan.schema()), ["n", "id", "name"]);
    }

    #[tokio::test]
    async fn history_reads_source_rows() {
        let plan = plan_sql("SELECT email, id FROM events").await;
        let history = plan.history();
        assert_eq!(
            history.full.sql(),
            "SELECT `id`, `email` FROM {t0:Identifier}"
        );
        assert_eq!(history.full.tables().collect::<Vec<_>>(), ["events"]);
        assert_eq!(history.full.schema(), &plan.sources()[0].schema);
        assert!(history.seed.is_none() && history.refill.is_none() && history.by_key.is_none());
    }

    #[tokio::test]
    async fn history_uses_history_expr() {
        let plan = plan_sql("SELECT * FROM dates").await;
        assert_eq!(
            plan.history().full.sql(),
            "SELECT `id`, toInt32(`d`) FROM {t0:Identifier}"
        );
    }

    #[tokio::test]
    async fn configured_topic_is_used() {
        let plan = plan_sql("SELECT * FROM dates").await;
        assert_eq!(plan.sources()[0].topic, "raw.dates");
    }

    #[tokio::test]
    async fn fingerprint_is_stable_and_meaningful() {
        let fingerprint = |sql: &'static str| async move { plan_sql(sql).await.fingerprint() };
        let star = fingerprint("SELECT * FROM events").await;
        assert_eq!(star, fingerprint("SELECT * FROM events").await);
        assert_eq!(
            star,
            fingerprint("SELECT id, name, email FROM events").await
        );
        for other in [
            "SELECT id AS key, name, email FROM events",
            "SELECT id, name FROM events",
            "SELECT name, id, email FROM events",
            "SELECT * FROM dates",
        ] {
            assert_ne!(star, fingerprint(other).await, "{other}");
        }
    }

    #[tokio::test]
    async fn statement_binds_clones() {
        let plan = plan_sql("SELECT id FROM events").await;
        let full = &plan.history().full;
        let clones = BTreeMap::from([("events".to_owned(), "live0__events__1".to_owned())]);
        let statement = full.fill(&[]).unwrap().statement(&clones).unwrap();
        assert_eq!(statement.sql, "SELECT `id` FROM {t0:Identifier}");
        assert_eq!(
            statement.params,
            [("t0".to_owned(), "live0__events__1".to_owned())]
        );
        assert!(full.statement(&BTreeMap::new()).is_err());
        assert!(full.fill(&[Value::Int(1)]).is_err());
    }

    #[tokio::test]
    async fn display_shows_the_plan() {
        let plan = plan_sql("SELECT name AS n, id FROM events").await;
        let shown = plan.to_string();
        assert!(
            shown.starts_with(&format!("append {}\n", plan.fingerprint())),
            "{shown}"
        );
        assert!(
            shown.contains("  read 0: events (topic events): id, name\n"),
            "{shown}"
        );
        assert!(
            shown.contains("  output: n = $0.name, id = $0.id\n"),
            "{shown}"
        );
        assert!(
            shown.contains("  history full: SELECT `id`, `name` FROM {t0:Identifier}\n"),
            "{shown}"
        );
    }
}
