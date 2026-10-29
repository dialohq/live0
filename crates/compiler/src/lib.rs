use serde_json::{Map, Value};

pub use crate::plan::Plan;
use crate::{bind::bind, plan::plan};

mod bind;
mod plan;

#[derive(Debug)]
pub enum Error {}

pub fn compile(query: parser::Query, params: Map<String, Value>) -> Result<Plan, Error> {
    let bound = bind(query, params).unwrap();
    let plan = plan(bound).unwrap();
    Ok(plan)
}
