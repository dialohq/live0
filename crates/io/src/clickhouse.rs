use async_trait::async_trait;
use serde_json::Value as Json;
use types::{DataType, DecodeError, Row, Schema};

use crate::{ColumnDefault, DescribedColumn, Error, HistorySource, Statement, TableDescription};

pub const HISTORY_FORMAT: &str = "JSONCompactEachRowWithNamesAndTypes";
pub const HISTORY_SETTINGS: &[(&str, &str)] = &[
    ("output_format_json_quote_64bit_integers", "1"),
    ("output_format_json_quote_denormals", "1"),
    ("date_time_output_format", "unix_timestamp"),
    ("output_format_json_quote_decimals", "1"),
];

#[derive(Debug, thiserror::Error)]
pub enum ClickhouseError {
    #[error("HTTP request error: {0}")]
    RequestError(#[from] reqwest::Error),
    #[error("ClickHouse returned {status}: {body}")]
    ServerError { status: u16, body: String },
    #[error("malformed response line: {0}")]
    ResponseError(#[from] serde_json::Error),
    #[error(transparent)]
    DecodeError(#[from] DecodeError),
}

/// A ClickHouse HTTP connection, also used by `snapshot` for clone DDL.
#[derive(Clone)]
pub struct ClickhouseClient {
    client: reqwest::Client,
    url: String,
    user: String,
    password: String,
    database: String,
}

impl ClickhouseClient {
    pub fn new(config: &config::ClickhouseConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            url: config.url.clone(),
            user: config.user.clone(),
            password: config.password.clone(),
            database: config.database.clone(),
        }
    }

    pub fn database(&self) -> &str {
        &self.database
    }

    /// Runs `statement` and returns the whole response body in `format`.
    pub async fn query(
        &self,
        statement: &Statement,
        format: &str,
    ) -> Result<String, ClickhouseError> {
        let params: Vec<(String, &str)> = statement
            .params
            .iter()
            .map(|(name, value)| (format!("param_{name}"), value.as_str()))
            .collect();
        let resp = self
            .client
            .post(&self.url)
            .header("X-ClickHouse-User", &self.user)
            .header("X-ClickHouse-Key", &self.password)
            .query(&[
                ("database", self.database.as_str()),
                ("default_format", format),
                // Otherwise an error after the first rows still comes back as 200.
                ("wait_end_of_query", "1"),
            ])
            .query(HISTORY_SETTINGS)
            .query(&params)
            .body(statement.sql.clone())
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(ClickhouseError::ServerError {
                status: status.as_u16(),
                body: body.trim().to_owned(),
            });
        }
        Ok(body)
    }

    /// Runs a statement without a result, such as DDL.
    pub async fn execute(&self, statement: &Statement) -> Result<(), ClickhouseError> {
        self.query(statement, "TabSeparated").await.map(drop)
    }
}

pub struct ClickhouseHistory {
    client: ClickhouseClient,
}

impl ClickhouseHistory {
    pub fn new(client: ClickhouseClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl HistorySource for ClickhouseHistory {
    async fn describe(&self, table: &str) -> Result<TableDescription, Error> {
        let columns = Statement {
            sql: "DESCRIBE TABLE {table:Identifier}".to_owned(),
            params: vec![("table".to_owned(), table.to_owned())],
        };
        let body = self.client.query(&columns, "JSONEachRow").await?;
        Ok(TableDescription {
            columns: decode_columns(&body)?,
            engine: todo!("get the table engine from `system.tables` as well"),
        })
    }

    async fn fetch(&self, statement: &Statement, schema: &Schema) -> Result<Vec<Row>, Error> {
        Ok(decode_rows(
            &self.client.query(statement, HISTORY_FORMAT).await?,
            schema,
        )?)
    }
}

fn decode_rows(body: &str, schema: &Schema) -> Result<Vec<Row>, ClickhouseError> {
    // Skips the names and types lines; the schema already says how to decode each column.
    body.lines()
        .filter(|l| !l.is_empty())
        .skip(2)
        .map(|line| Ok(types::decode_history(&serde_json::from_str(line)?, schema)?))
        .collect()
}

fn decode_columns(body: &str) -> Result<Vec<DescribedColumn>, ClickhouseError> {
    body.lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let row: Json = serde_json::from_str(line)?;
            let field = |key: &str| row[key].as_str().unwrap_or_default();
            Ok(DescribedColumn {
                name: field("name").to_owned(),
                ty: DataType::parse(field("type")),
                default: match field("default_type") {
                    "DEFAULT" => ColumnDefault::Default,
                    "MATERIALIZED" => ColumnDefault::Materialized,
                    "ALIAS" => ColumnDefault::Alias,
                    "EPHEMERAL" => ColumnDefault::Ephemeral,
                    _ => ColumnDefault::None,
                },
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use types::{Column, Value};

    use super::*;

    fn schema(columns: &[(&str, &str)]) -> Schema {
        Schema {
            columns: columns
                .iter()
                .map(|(name, ty)| Column::new(*name, DataType::parse(ty)))
                .collect(),
        }
    }

    #[test]
    fn decodes_rows_by_schema() {
        let body = concat!(
            "[\"id\", \"name\", \"score\"]\n",
            "[\"UInt64\", \"LowCardinality(String)\", \"Nullable(Float64)\"]\n",
            "[\"18446744073709551615\", \"a\", 1.5]\n",
            "[\"0\", \"\", null]\n",
        );
        let schema = schema(&[
            ("id", "UInt64"),
            ("name", "LowCardinality(String)"),
            ("score", "Nullable(Float64)"),
        ]);
        assert_eq!(
            decode_rows(body, &schema).unwrap(),
            vec![
                vec![
                    Value::Int(u64::MAX.into()),
                    Value::String("a".to_owned()),
                    Value::Float(1.5)
                ],
                vec![Value::Int(0), Value::String(String::new()), Value::Null],
            ]
        );
    }

    #[test]
    fn no_rows() {
        let schema = schema(&[("a", "Int8")]);
        assert_eq!(
            decode_rows("[\"a\"]\n[\"Int8\"]\n", &schema).unwrap(),
            Vec::<Row>::new()
        );
        assert_eq!(decode_rows("", &schema).unwrap(), Vec::<Row>::new());
    }

    #[test]
    fn invalid_history_row() {
        let schema = schema(&[("a", "Int8")]);
        let err = decode_rows("[\"a\"]\n[\"Int8\"]\n[300]\n", &schema).unwrap_err();
        assert!(matches!(
            err,
            ClickhouseError::DecodeError(DecodeError::Value { .. })
        ));
    }

    #[test]
    fn describe_keeps_every_column() {
        let body = concat!(
            "{\"name\":\"id\",\"type\":\"UUID\",\"default_type\":\"\"}\n",
            "{\"name\":\"name\",\"type\":\"LowCardinality(Nullable(String))\",\"default_type\":\"DEFAULT\"}\n",
            "{\"name\":\"upper\",\"type\":\"String\",\"default_type\":\"MATERIALIZED\"}\n",
        );
        let column = |name: &str, ty: &str, default| DescribedColumn {
            name: name.to_owned(),
            ty: DataType::parse(ty),
            default,
        };
        assert_eq!(
            decode_columns(body).unwrap(),
            vec![
                column("id", "UUID", ColumnDefault::None),
                column("name", "Nullable(String)", ColumnDefault::Default),
                column("upper", "String", ColumnDefault::Materialized),
            ]
        );
    }
}
