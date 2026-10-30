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

fn error(message: impl Into<String>) -> Error {
    Error {
        message: message.into(),
        span: None,
    }
}

impl HistorySql {
    pub(crate) fn new(
        sql: String,
        params: Vec<(String, String)>,
        tables: BTreeMap<String, String>,
        slots: Vec<String>,
        schema: Schema,
    ) -> Self {
        Self {
            sql,
            params,
            tables,
            slots,
            schema,
        }
    }

    pub(crate) fn sql(&self) -> &str {
        &self.sql
    }

    /// How each result row decodes.
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    pub fn tables(&self) -> impl Iterator<Item = &str> {
        self.tables.keys().map(String::as_str)
    }

    /// Fills the slots, in order.
    pub fn fill(&self, values: &[Value]) -> Result<HistorySql, Error> {
        if values.len() != self.slots.len() {
            return Err(error(format!(
                "history query has {} slots, got {} values",
                self.slots.len(),
                values.len()
            )));
        }
        if !values.is_empty() {
            return Err(error("filling slots isn't supported yet"));
        }
        Ok(self.clone())
    }

    /// Binds each table to its clone; fails if a table or a slot is left unbound.
    pub fn statement(&self, clones: &BTreeMap<String, String>) -> Result<io::Statement, Error> {
        if let Some(slot) = self.slots.first() {
            return Err(error(format!("history query slot `{slot}` isn't filled")));
        }
        let mut params = self.params.clone();
        for (table, param) in &self.tables {
            let clone = clones.get(table).ok_or_else(|| {
                error(format!("no clone of `{table}` to run the history query on"))
            })?;
            params.push((param.clone(), clone.clone()));
        }
        Ok(io::Statement {
            sql: self.sql.clone(),
            params,
        })
    }
}
