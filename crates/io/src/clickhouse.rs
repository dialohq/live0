use async_trait::async_trait;
use serde_json::Value as Json;
use types::{DataType, DecodeError, DecodeSpec, Row};

use crate::{DescribedTable, HISTORY_FORMAT, HISTORY_SETTINGS, HistorySource};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("HTTP request error: {0}")]
    RequestError(#[from] reqwest::Error),
    #[error("ClickHouse returned {status}: {body}")]
    ServerError { status: u16, body: String },
    #[error("malformed response line: {0}")]
    ResponseError(#[from] serde_json::Error),
    #[error("column `{column}` has an unsupported type `{ty}`")]
    UnsupportedType { column: String, ty: String },
    #[error(transparent)]
    DecodeError(#[from] DecodeError),
}

pub struct ClickhouseHistory {
    client: reqwest::Client,
    host: String,
    port: u16,
    user: String,
    password: String,
    database: String,
}

impl ClickhouseHistory {
    pub fn new(host: String, port: u16, user: String, password: String, database: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            host,
            port,
            user,
            password,
            database,
        }
    }

    async fn query(&self, sql: &str, format: &str) -> Result<String, Error> {
        let resp = self
            .client
            .post(format!("http://{}:{}/", self.host, self.port))
            .header("X-ClickHouse-User", &self.user)
            .header("X-ClickHouse-Key", &self.password)
            .query(&[
                ("database", self.database.as_str()),
                ("default_format", format),
                // Otherwise an error after the first rows still comes back as 200.
                ("wait_end_of_query", "1"),
            ])
            .query(HISTORY_SETTINGS)
            .body(sql.to_owned())
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(Error::ServerError {
                status: status.as_u16(),
                body: body.trim().to_owned(),
            });
        }
        Ok(body)
    }
}

#[async_trait]
impl HistorySource for ClickhouseHistory {
    type Error = Error;

    async fn fetch(&self, spec: DecodeSpec, sql: &str) -> Result<Vec<Row>, Error> {
        decode_rows(&self.query(sql, HISTORY_FORMAT).await?, &spec)
    }

    async fn describe(&self, table: &str) -> Result<DescribedTable, Error> {
        let body = self
            .query(&format!("DESCRIBE TABLE {table}"), "JSONEachRow")
            .await?;
        decode_describe(&body)
    }
}

fn decode_rows(body: &str, spec: &DecodeSpec) -> Result<Vec<Row>, Error> {
    // Skips the names and types lines; the spec already says how to decode each column.
    body.lines()
        .filter(|l| !l.is_empty())
        .skip(2)
        .map(|line| Ok(types::decode_history(&serde_json::from_str(line)?, spec)?))
        .collect()
}

fn decode_describe(body: &str) -> Result<DescribedTable, Error> {
    let mut spec = DecodeSpec::new();
    for line in body.lines().filter(|l| !l.is_empty()) {
        let row: Json = serde_json::from_str(line)?;
        let field = |key: &str| row[key].as_str().unwrap_or_default().to_owned();
        // Neither selected by `SELECT *` nor written by inserts.
        if matches!(
            field("default_type").as_str(),
            "MATERIALIZED" | "ALIAS" | "EPHEMERAL"
        ) {
            continue;
        }
        let (name, ty) = (field("name"), field("type"));
        match DataType::parse(&ty) {
            DataType::Other(_) => return Err(Error::UnsupportedType { column: name, ty }),
            parsed => spec.push((name, parsed)),
        }
    }

    Ok(DescribedTable {
        columns: spec,
        engine: todo!("get the table engine from `system.tables` as well"),
    })
}

#[cfg(test)]
mod tests {
    use types::Value;

    use super::*;

    fn spec(columns: &[(&str, &str)]) -> DecodeSpec {
        columns
            .iter()
            .map(|(name, ty)| ((*name).to_owned(), DataType::parse(ty)))
            .collect()
    }

    #[test]
    fn decodes_rows_by_spec() {
        let body = concat!(
            "[\"id\", \"name\", \"score\"]\n",
            "[\"UInt64\", \"LowCardinality(String)\", \"Nullable(Float64)\"]\n",
            "[\"18446744073709551615\", \"a\", 1.5]\n",
            "[\"0\", \"\", null]\n",
        );
        let spec = spec(&[
            ("id", "UInt64"),
            ("name", "LowCardinality(String)"),
            ("score", "Nullable(Float64)"),
        ]);
        assert_eq!(
            decode_rows(body, &spec).unwrap(),
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
        let spec = spec(&[("a", "Int8")]);
        assert_eq!(
            decode_rows("[\"a\"]\n[\"Int8\"]\n", &spec).unwrap(),
            Vec::<Row>::new()
        );
        assert_eq!(decode_rows("", &spec).unwrap(), Vec::<Row>::new());
    }

    #[test]
    fn invalid_history_row() {
        let spec = spec(&[("a", "Int8")]);
        let err = decode_rows("[\"a\"]\n[\"Int8\"]\n[300]\n", &spec).unwrap_err();
        assert!(matches!(err, Error::DecodeError(DecodeError::Value { .. })));
    }

    #[test]
    fn describe_skips_unstored_columns() {
        let body = concat!(
            "{\"name\":\"id\",\"type\":\"UInt64\",\"default_type\":\"\"}\n",
            "{\"name\":\"name\",\"type\":\"LowCardinality(Nullable(String))\",\"default_type\":\"DEFAULT\"}\n",
            "{\"name\":\"upper\",\"type\":\"String\",\"default_type\":\"MATERIALIZED\"}\n",
            "{\"name\":\"alias\",\"type\":\"String\",\"default_type\":\"ALIAS\"}\n",
        );
        assert_eq!(
            decode_describe(body).unwrap(),
            spec(&[("id", "UInt64"), ("name", "Nullable(String)")])
        );
    }

    #[test]
    fn describe_unsupported_type() {
        let body = "{\"name\":\"id\",\"type\":\"UUID\",\"default_type\":\"\"}\n";
        assert!(matches!(
            decode_describe(body),
            Err(Error::UnsupportedType { column, ty }) if column == "id" && ty == "UUID"
        ));
    }
}
