use catalog::Catalog;

use crate::{Access, Error, Params};

/// The query with names resolved, types checked, parameters bound, security applied and normalized.
pub(crate) struct BoundQuery {}

pub(crate) fn bind(
    query: &parser::Query,
    catalog: &Catalog,
    params: &Params,
    access: &Access,
) -> Result<BoundQuery, Error> {
    todo!()
}
