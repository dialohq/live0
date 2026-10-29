use types::{DataType, Value};

use crate::aggregate::{nullable, wrap_nullable};
use crate::{Aggregate, AggregateState, Error, EvalError, Seed};

#[derive(Debug)]
pub(super) struct Sum {
    zero: Total,
    nullable: bool,
}

impl Sum {
    pub(super) fn new(args: &[DataType]) -> Result<Sum, Error> {
        let (zero, nullable) = Total::bind("sum", args)?;
        Ok(Sum { zero, nullable })
    }
}

impl Aggregate for Sum {
    fn result_type(&self) -> DataType {
        wrap_nullable(self.zero.ty(), self.nullable)
    }

    fn seed(&self, args: &[&str]) -> Vec<Seed> {
        Running::seed(args, self.result_type())
    }

    fn state(&self) -> Box<dyn AggregateState> {
        Box::new(SumState {
            running: Running::new(self.zero),
            nullable: self.nullable,
        })
    }

    fn seeded(&self, seed: &[Value]) -> Result<Box<dyn AggregateState>, EvalError> {
        Ok(Box::new(SumState {
            running: Running::seeded("sum", self.zero, seed)?,
            nullable: self.nullable,
        }))
    }
}

#[derive(Debug)]
struct SumState {
    running: Running,
    nullable: bool,
}

impl AggregateState for SumState {
    fn add(&mut self, args: &[Value]) -> Result<(), EvalError> {
        self.running.add("sum", args)
    }

    fn remove(&mut self, args: &[Value]) -> Result<bool, EvalError> {
        self.running.remove("sum", args)
    }

    fn value(&self) -> Value {
        match self.nullable && self.running.count == 0 {
            true => Value::Null,
            false => self.running.total.value(),
        }
    }
}

/// The total and the count of non-`NULL` values; `NULL`s are skipped, as ClickHouse does.
#[derive(Debug)]
pub(super) struct Running {
    pub(super) total: Total,
    pub(super) count: u64,
}

impl Running {
    pub(super) fn new(zero: Total) -> Running {
        Running {
            total: zero,
            count: 0,
        }
    }

    /// `sum(x)` typed `ty`, then `count(x)`.
    pub(super) fn seed(args: &[&str], ty: DataType) -> Vec<Seed> {
        let arg = args.first().copied().unwrap_or_default();
        vec![
            Seed {
                sql: format!("sum({arg})"),
                ty,
            },
            Seed {
                sql: format!("count({arg})"),
                ty: DataType::parse("UInt64"),
            },
        ]
    }

    pub(super) fn seeded(
        function: &'static str,
        zero: Total,
        seed: &[Value],
    ) -> Result<Running, EvalError> {
        let mut running = Running::new(zero);
        match seed {
            [Value::Null, Value::Int(0)] => {}
            [sum, Value::Int(count)] => {
                running.total.add(function, sum)?;
                running.count = u64::try_from(*count).map_err(|_| EvalError::Mismatch {
                    function,
                    value: Value::Int(*count),
                })?;
            }
            _ => {
                return Err(EvalError::Mismatch {
                    function,
                    value: seed.first().cloned().unwrap_or(Value::Null),
                });
            }
        }
        Ok(running)
    }

    pub(super) fn add(&mut self, function: &'static str, args: &[Value]) -> Result<(), EvalError> {
        match single(function, args)? {
            Value::Null => Ok(()),
            value => {
                self.total.add(function, value)?;
                self.count += 1;
                Ok(())
            }
        }
    }

    pub(super) fn remove(
        &mut self,
        function: &'static str,
        args: &[Value],
    ) -> Result<bool, EvalError> {
        match single(function, args)? {
            Value::Null => Ok(true),
            value => {
                let removed = self.total.remove(function, value)?;
                if removed {
                    self.count = self.count.saturating_sub(1);
                }
                Ok(removed)
            }
        }
    }
}

/// A total in the type ClickHouse sums the argument into, wrapping on overflow like it does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Total {
    Int(i64),
    UInt(u64),
    Float(f64),
}

impl Total {
    /// The zero total for the single argument, and whether the argument is nullable.
    pub(super) fn bind(function: &'static str, args: &[DataType]) -> Result<(Total, bool), Error> {
        let zero = match args {
            [arg] => match nullable(arg) {
                (DataType::Int(i), _) if i.width() > 64 => None,
                (DataType::Int(i), nullable) if i.is_signed() => Some((Total::Int(0), nullable)),
                (DataType::Int(_), nullable) => Some((Total::UInt(0), nullable)),
                (DataType::Float(_), nullable) => Some((Total::Float(0.0), nullable)),
                _ => None,
            },
            _ => None,
        };
        zero.ok_or_else(|| Error::Arguments {
            function,
            args: args.to_vec(),
        })
    }

    pub(super) fn ty(&self) -> DataType {
        DataType::parse(match self {
            Total::Int(_) => "Int64",
            Total::UInt(_) => "UInt64",
            Total::Float(_) => "Float64",
        })
    }

    pub(super) fn value(&self) -> Value {
        match *self {
            Total::Int(t) => Value::Int(i128::from(t)),
            Total::UInt(t) => Value::Int(i128::from(t)),
            Total::Float(t) => Value::Float(t),
        }
    }

    pub(super) fn as_f64(&self) -> f64 {
        match *self {
            Total::Int(t) => t as f64,
            Total::UInt(t) => t as f64,
            Total::Float(t) => t,
        }
    }

    fn add(&mut self, function: &'static str, value: &Value) -> Result<(), EvalError> {
        match (self, value) {
            (Total::Int(t), Value::Int(n)) if let Ok(n) = i64::try_from(*n) => {
                *t = t.wrapping_add(n);
            }
            (Total::UInt(t), Value::Int(n)) if let Ok(n) = u64::try_from(*n) => {
                *t = t.wrapping_add(n);
            }
            (Total::Float(t), Value::Float(f)) => *t += f,
            _ => return Err(mismatch(function, value)),
        }
        Ok(())
    }

    /// Subtraction undoes wrapping integer addition exactly, but not a float total that isn't finite.
    fn remove(&mut self, function: &'static str, value: &Value) -> Result<bool, EvalError> {
        match (self, value) {
            (Total::Int(t), Value::Int(n)) if let Ok(n) = i64::try_from(*n) => {
                *t = t.wrapping_sub(n);
            }
            (Total::UInt(t), Value::Int(n)) if let Ok(n) = u64::try_from(*n) => {
                *t = t.wrapping_sub(n);
            }
            (Total::Float(t), Value::Float(f)) if !t.is_finite() || !f.is_finite() => {
                return Ok(false);
            }
            (Total::Float(t), Value::Float(f)) => *t -= f,
            _ => return Err(mismatch(function, value)),
        }
        Ok(true)
    }
}

fn single<'a>(function: &'static str, args: &'a [Value]) -> Result<&'a Value, EvalError> {
    match args {
        [value] => Ok(value),
        _ => Err(mismatch(function, args.first().unwrap_or(&Value::Null))),
    }
}

fn mismatch(function: &'static str, value: &Value) -> EvalError {
    EvalError::Mismatch {
        function,
        value: value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregate;

    fn sum(ty: &str) -> std::sync::Arc<dyn Aggregate> {
        aggregate("SUM", &[DataType::parse(ty)]).unwrap()
    }

    #[test]
    fn result_types() {
        assert_eq!(sum("Int8").result_type(), DataType::parse("Int64"));
        assert_eq!(sum("UInt32").result_type(), DataType::parse("UInt64"));
        assert_eq!(sum("Float32").result_type(), DataType::parse("Float64"));
        assert_eq!(
            sum("Nullable(Int32)").result_type(),
            DataType::parse("Nullable(Int64)")
        );
        assert!(aggregate("sum", &[DataType::parse("String")]).is_err());
        assert!(aggregate("sum", &[]).is_err());
    }

    #[test]
    fn wraps_and_takes_back() {
        let mut state = sum("Int64").state();
        state.add(&[Value::Int(i128::from(i64::MAX))]).unwrap();
        state.add(&[Value::Int(1)]).unwrap();
        assert_eq!(state.value(), Value::Int(i128::from(i64::MIN)));
        assert!(state.remove(&[Value::Int(1)]).unwrap());
        assert_eq!(state.value(), Value::Int(i128::from(i64::MAX)));
    }

    #[test]
    fn nullable_is_null_without_values() {
        let mut state = sum("Nullable(Int32)").state();
        assert_eq!(state.value(), Value::Null);
        state.add(&[Value::Null]).unwrap();
        assert_eq!(state.value(), Value::Null);
        state.add(&[Value::Int(5)]).unwrap();
        assert_eq!(state.value(), Value::Int(5));
        assert!(state.remove(&[Value::Int(5)]).unwrap());
        assert_eq!(state.value(), Value::Null);
        assert_eq!(sum("Int32").state().value(), Value::Int(0));
    }

    #[test]
    fn infinite_float_cant_be_taken_back() {
        let mut state = sum("Float64").state();
        state.add(&[Value::Float(f64::INFINITY)]).unwrap();
        assert!(!state.remove(&[Value::Float(f64::INFINITY)]).unwrap());
    }

    #[test]
    fn seeds_from_sum_and_count() {
        let agg = sum("Nullable(UInt8)");
        let seed = agg.seed(&["x"]);
        assert_eq!(seed[0].sql, "sum(x)");
        assert_eq!(seed[1].sql, "count(x)");
        let mut state = agg.seeded(&[Value::Int(10), Value::Int(2)]).unwrap();
        state.add(&[Value::Int(3)]).unwrap();
        assert_eq!(state.value(), Value::Int(13));
        let empty = agg.seeded(&[Value::Null, Value::Int(0)]).unwrap();
        assert_eq!(empty.value(), Value::Null);
    }
}
