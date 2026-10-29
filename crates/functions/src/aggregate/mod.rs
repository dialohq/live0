use std::{fmt, sync::Arc};

use types::{DataType, Value};

use crate::{Error, EvalError};

mod avg;
mod sum;

/// An aggregate function bound to its argument types.
pub trait Aggregate: fmt::Debug + Send + Sync {
    fn result_type(&self) -> DataType;
    /// The history columns a group's state is seeded from, given each argument's SQL.
    fn seed(&self, args: &[&str]) -> Vec<Seed>;
    /// The state of a group with no rows.
    fn state(&self) -> Box<dyn AggregateState>;
    /// The state of a group from its seed columns' values, in `seed` order.
    fn seeded(&self, seed: &[Value]) -> Result<Box<dyn AggregateState>, EvalError>;
}

/// The running value of one group.
pub trait AggregateState: fmt::Debug + Send {
    fn add(&mut self, args: &[Value]) -> Result<(), EvalError>;
    /// `false` when the row can't be taken back, so the group has to be seeded again.
    fn remove(&mut self, args: &[Value]) -> Result<bool, EvalError>;
    fn value(&self) -> Value;
}

/// A column of the history query that seeds an aggregate's state.
#[derive(Debug, Clone, PartialEq)]
pub struct Seed {
    pub sql: String,
    pub ty: DataType,
}

/// Looks an aggregate up by name, case-insensitively like ClickHouse, and binds it to its argument types.
pub fn aggregate(name: &str, args: &[DataType]) -> Result<Arc<dyn Aggregate>, Error> {
    match name.to_ascii_lowercase().as_str() {
        "sum" => Ok(Arc::new(sum::Sum::new(args)?)),
        "avg" => Ok(Arc::new(avg::Avg::new(args)?)),
        _ => Err(Error::UnknownAggregate(name.to_owned())),
    }
}

/// The type inside `Nullable`, and whether it was there.
fn nullable(ty: &DataType) -> (&DataType, bool) {
    match ty {
        DataType::Nullable(inner) => (inner, true),
        other => (other, false),
    }
}

fn wrap_nullable(ty: DataType, nullable: bool) -> DataType {
    match nullable {
        true => DataType::Nullable(Box::new(ty)),
        false => ty,
    }
}
