use types::{DataType, Value};

use crate::aggregate::sum::{Running, Total};
use crate::aggregate::wrap_nullable;
use crate::{Aggregate, AggregateState, Error, EvalError, Seed};

/// Sums into the same type as `sum`, then divides as `Float64`.
#[derive(Debug)]
pub(super) struct Avg {
    zero: Total,
    nullable: bool,
}

impl Avg {
    pub(super) fn new(args: &[DataType]) -> Result<Avg, Error> {
        let (zero, nullable) = Total::bind("avg", args)?;
        Ok(Avg { zero, nullable })
    }
}

impl Aggregate for Avg {
    fn result_type(&self) -> DataType {
        wrap_nullable(DataType::parse("Float64"), self.nullable)
    }

    fn seed(&self, args: &[&str]) -> Vec<Seed> {
        Running::seed(args, wrap_nullable(self.zero.ty(), self.nullable))
    }

    fn state(&self) -> Box<dyn AggregateState> {
        Box::new(AvgState {
            running: Running::new(self.zero),
            nullable: self.nullable,
        })
    }

    fn seeded(&self, seed: &[Value]) -> Result<Box<dyn AggregateState>, EvalError> {
        Ok(Box::new(AvgState {
            running: Running::seeded("avg", self.zero, seed)?,
            nullable: self.nullable,
        }))
    }
}

#[derive(Debug)]
struct AvgState {
    running: Running,
    nullable: bool,
}

impl AggregateState for AvgState {
    fn add(&mut self, args: &[Value]) -> Result<(), EvalError> {
        self.running.add("avg", args)
    }

    fn remove(&mut self, args: &[Value]) -> Result<bool, EvalError> {
        self.running.remove("avg", args)
    }

    /// `NaN` for no rows, or `NULL` if the argument is nullable.
    fn value(&self) -> Value {
        match (self.running.count, self.nullable) {
            (0, true) => Value::Null,
            (count, _) => Value::Float(self.running.total.as_f64() / count as f64),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregate;

    fn avg(ty: &str) -> std::sync::Arc<dyn Aggregate> {
        aggregate("avg", &[DataType::parse(ty)]).unwrap()
    }

    #[test]
    fn result_types() {
        assert_eq!(avg("Int8").result_type(), DataType::parse("Float64"));
        assert_eq!(
            avg("Nullable(UInt64)").result_type(),
            DataType::parse("Nullable(Float64)")
        );
    }

    #[test]
    fn averages_and_takes_back() {
        let mut state = avg("Int32").state();
        for n in [1, 2, 6] {
            state.add(&[Value::Int(n)]).unwrap();
        }
        assert_eq!(state.value(), Value::Float(3.0));
        assert!(state.remove(&[Value::Int(6)]).unwrap());
        assert_eq!(state.value(), Value::Float(1.5));
    }

    #[test]
    fn empty() {
        let Value::Float(f) = avg("Float64").state().value() else {
            panic!("not a float");
        };
        assert!(f.is_nan());
        assert_eq!(avg("Nullable(Float64)").state().value(), Value::Null);
    }

    #[test]
    fn seeded() {
        let agg = avg("UInt16");
        assert_eq!(agg.seed(&["x"])[0].ty, DataType::parse("UInt64"));
        let state = agg.seeded(&[Value::Int(9), Value::Int(3)]).unwrap();
        assert_eq!(state.value(), Value::Float(3.0));
    }
}
