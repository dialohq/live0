use compiler::Plan;
use serde_json::{Map, Value};

#[derive(Debug)]
pub enum Error {}

// for security, probably doesn't belong here
struct RowFilter {}

pub struct Engine {}

pub struct Subscription {}

impl Engine {
    pub async fn new(config: config::Config) -> Result<Self, Error> {
        todo!()
    }

    pub async fn explain(&self, sql: &str, params: Map<String, Value>) -> Plan {
        let parsed = parser::parse(sql).unwrap();
        let plan = compiler::compile(parsed, params).unwrap();
        plan
    }

    pub async fn subscribe(
        &self,
        sql: &str,
        params: Map<String, Value>,
        filters: Vec<RowFilter>,
    ) -> Subscription {
        let plan = self.explain(sql, params).await;
        todo!()
    }
}

impl Subscription {}
