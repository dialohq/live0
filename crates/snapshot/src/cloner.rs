use std::time::{Duration, UNIX_EPOCH};

use async_trait::async_trait;
use io::{ClickhouseClient, Statement, quote_identifier};
use serde_json::{Map, Value as Json, json};
use types::Watermark;

use crate::{CloneInfo, Cloner, Error};

/// `CREATE TABLE … AS` plus `ATTACH PARTITION ALL FROM`, with the clone's info in its table comment.
pub struct ClickhouseCloner {
    client: ClickhouseClient,
}

impl ClickhouseCloner {
    pub fn new(client: ClickhouseClient) -> Self {
        Self { client }
    }

    async fn run(&self, sql: String) -> Result<(), Error> {
        self.client
            .execute(&Statement {
                sql,
                params: Vec::new(),
            })
            .await
            .map_err(|err| Error::Clone(err.to_string()))
    }

    async fn fill(&self, clone: &CloneInfo, name: &str) -> Result<(), Error> {
        let table = quote_identifier(&clone.table);
        self.run(format!(
            "ALTER TABLE {name} ATTACH PARTITION ALL FROM {table}"
        ))
        .await?;
        let comment = quote_string(&comment(clone).to_string());
        self.run(format!("ALTER TABLE {name} MODIFY COMMENT {comment}"))
            .await
    }
}

/// A ClickHouse string literal.
fn quote_string(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn comment(clone: &CloneInfo) -> Json {
    let watermark: Map<String, Json> = clone
        .watermark
        .topics()
        .map(|(topic, offsets)| {
            let offsets = offsets
                .iter()
                .map(|(partition, offset)| (partition.to_string(), json!(offset)))
                .collect();
            (topic.to_owned(), Json::Object(offsets))
        })
        .collect();
    let taken_at_ms = clone
        .taken_at
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    json!({ "table": clone.table, "watermark": watermark, "taken_at_ms": taken_at_ms })
}

/// The clone's info from its comment; `None` for tables live0 didn't clone.
fn parse_comment(name: &str, comment: &str) -> Option<CloneInfo> {
    let json: Json = serde_json::from_str(comment).ok()?;
    let mut watermark = Watermark::new();
    for (topic, offsets) in json.get("watermark")?.as_object()? {
        for (partition, offset) in offsets.as_object()? {
            watermark.set(topic, partition.parse().ok()?, offset.as_i64()?);
        }
    }
    Some(CloneInfo {
        name: name.to_owned(),
        table: json.get("table")?.as_str()?.to_owned(),
        watermark,
        taken_at: UNIX_EPOCH + Duration::from_millis(json.get("taken_at_ms")?.as_u64()?),
    })
}

#[async_trait]
impl Cloner for ClickhouseCloner {
    async fn create(&self, clone: &CloneInfo) -> Result<(), Error> {
        let name = quote_identifier(&clone.name);
        let table = quote_identifier(&clone.table);
        self.run(format!("CREATE TABLE {name} AS {table}")).await?;
        let filled = self.fill(clone, &name).await;
        if filled.is_err() {
            let _ = Cloner::drop(self, &clone.name).await;
        }
        filled
    }

    async fn drop(&self, name: &str) -> Result<(), Error> {
        self.run(format!("DROP TABLE IF EXISTS {}", quote_identifier(name)))
            .await
    }

    async fn list(&self, namespace: &str) -> Result<Vec<CloneInfo>, Error> {
        let statement = Statement {
            sql: "SELECT name, comment FROM system.tables \
                  WHERE database = currentDatabase() AND startsWith(name, {prefix:String})"
                .to_owned(),
            params: vec![("prefix".to_owned(), format!("{namespace}__"))],
        };
        let body = self
            .client
            .query(&statement, "JSONEachRow")
            .await
            .map_err(|err| Error::Clone(err.to_string()))?;
        Ok(body
            .lines()
            .filter_map(|line| serde_json::from_str::<Json>(line).ok())
            .filter_map(|row| {
                parse_comment(row.get("name")?.as_str()?, row.get("comment")?.as_str()?)
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comment_round_trips() {
        let mut watermark = Watermark::new();
        watermark.set("events", 0, 5);
        watermark.set("events", 2, 7);
        let clone = CloneInfo {
            name: "live0__events__1_1".to_owned(),
            table: "events".to_owned(),
            watermark,
            taken_at: UNIX_EPOCH + Duration::from_millis(1_700_000_000_123),
        };
        let text = comment(&clone).to_string();
        assert_eq!(parse_comment(&clone.name, &text), Some(clone));
        assert_eq!(parse_comment("x", "a user comment"), None);
        assert_eq!(quote_string("it's \\ fine"), "'it\\'s \\\\ fine'");
    }
}
