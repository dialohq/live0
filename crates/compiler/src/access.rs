use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value as Json};

/// Values for the query's `{name:Type}` parameters.
pub type Params = Map<String, Json>;

/// What the subscribing subject is allowed to see.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Access {
    /// Security rules don't apply, as when no permission provider is configured.
    Unrestricted,
    /// Grants the subject doesn't have are missing from the map.
    Grants(BTreeMap<String, Grant>),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Grant {
    Check(bool),
    /// Resource external ids, compared with a row rule's column.
    Values(BTreeSet<String>),
}
