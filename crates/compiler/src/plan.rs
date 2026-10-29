use crate::bind::BoundQuery;

#[derive(Debug, thiserror::Error)]
pub enum Error {}

pub struct Plan {}

pub fn plan(query: BoundQuery) -> Result<Plan, Error> {
    todo!()
}
