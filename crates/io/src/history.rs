use async_trait::async_trait;
use types::{DataType, Row, Schema};

use crate::Error;

/// SQL with its values passed separately, as ClickHouse `{name:Type}` query parameters.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Statement {
    pub sql: String,
    /// Each parameter's value in ClickHouse's text form; table names are `Identifier` parameters.
    pub params: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableDescription {
    pub columns: Vec<DescribedColumn>,
    pub engine: TableEngine,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DescribedColumn {
    pub name: String,
    /// `DataType::Other` for types live0 doesn't support; only an error once a query reads it.
    pub ty: DataType,
    pub default: ColumnDefault,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnDefault {
    None,
    Default,
    Materialized,
    Alias,
    Ephemeral,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableEngine {
    MergeTree,
    ReplacingMergeTree(ReplacingMergeTree),
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplacingMergeTree {
    /// The `ORDER BY` columns, which identify a row's key.
    pub key: Vec<String>,
    pub version: Option<String>,
    pub is_deleted: Option<String>,
}

#[async_trait]
pub trait HistorySource: Send + Sync {
    async fn describe(&self, table: &str) -> Result<TableDescription, Error>;

    /// Runs `statement`, whose tables are already bound to clones, decoding each result row by `schema`.
    async fn fetch(&self, statement: &Statement, schema: &Schema) -> Result<Vec<Row>, Error>;
}
