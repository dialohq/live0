use std::collections::BTreeMap;

use types::{Schema, Value};

use crate::Error;

/// The history queries a plan can need, all against clones of its sources.
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryQueries {
    /// The current result, which a fresh query starts from.
    pub full: HistorySql,
    /// The state a resumed query needs besides its checkpoint, for shapes that need one.
    pub seed: Option<HistorySql>,
    /// Rows past the last shown sort key, to refill a window that deletes drained.
    pub refill: Option<HistorySql>,
    /// Outer rows for keys that just arrived in a semi-join's inner set.
    pub by_key: Option<HistorySql>,
}

/// Rendered SQL whose values are query parameters and whose tables are bound to clones when it runs.
#[derive(Debug, Clone, PartialEq)]
pub struct HistorySql {
    sql: String,
    params: Vec<(String, String)>,
    /// `Identifier` parameter name per table name.
    tables: BTreeMap<String, String>,
    /// Parameters `qstate` fills in when it asks for the fetch, such as a refill's last sort key.
    slots: Vec<String>,
    schema: Schema,
}

impl HistorySql {
    /// How each result row decodes.
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    pub fn tables(&self) -> impl Iterator<Item = &str> {
        self.tables.keys().map(String::as_str)
    }

    /// Fills the slots, in order.
    pub fn fill(&self, values: &[Value]) -> Result<HistorySql, Error> {
        todo!()
    }

    /// Binds each table to its clone; fails if a table or a slot is left unbound.
    pub fn statement(&self, clones: &BTreeMap<String, String>) -> Result<io::Statement, Error> {
        todo!()
    }
}
