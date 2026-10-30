pub use crate::access::{Access, Grant, Params};
pub use crate::error::Error;
pub use crate::expr::{CompareOp, Expr, Function};
pub use crate::history::{HistoryQueries, HistorySql};
pub use crate::plan::{Aggregate, Fingerprint, Plan, Shape, SortKey, Source, SourceId, TimeBound};

mod access;
mod bind;
mod error;
mod expr;
mod history;
mod plan;
#[cfg(test)]
mod testing;

pub fn compile(
    query: &parser::Query,
    catalog: &catalog::Catalog,
    params: &Params,
    access: &Access,
) -> Result<Plan, Error> {
    let bound = bind::bind(query, catalog, params, access)?;
    plan::plan(bound)
}
