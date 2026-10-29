use serde_json::{Map, Value};

#[derive(Debug, thiserror::Error)]
pub enum Error {}

pub struct BoundQuery {}

pub fn bind(parsed: parser::ast::Query, params: Map<String, Value>) -> Result<BoundQuery, Error> {
    todo!()
}
