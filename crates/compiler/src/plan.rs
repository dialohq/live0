use std::{fmt, sync::Arc, time::Duration};

use types::Schema;

use crate::{CompareOp, Error, Expr, HistoryQueries, bind::BoundQuery};

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
        todo!()
    }
}

pub(crate) fn plan(query: BoundQuery) -> Result<Plan, Error> {
    todo!()
}
