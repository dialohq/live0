mod common;

use std::net::Ipv4Addr;

use common::{Database, clickhouse_config, statement};
use ethnum::I256;
use io::{
    ClickhouseClient, ClickhouseError, ClickhouseHistory, ColumnDefault, DescribedColumn, Error,
    HistorySource, ReplacingMergeTree, Statement, TableEngine,
};
use serde_json::{Value as Json, json};
use types::{Column, DataType, Row, Schema, Value};

fn history(db: &Database) -> ClickhouseHistory {
    ClickhouseHistory::new(db.client.clone())
}

/// Selects every column of `table` the way the compiler will, by its `history_expr`.
async fn fetch_all(db: &Database, table: &str) -> Result<Vec<Row>, Error> {
    let history = history(db);
    let schema = Schema {
        columns: history
            .describe(table)
            .await?
            .columns
            .into_iter()
            .map(|c| Column::new(c.name, c.ty))
            .collect(),
    };
    let exprs: Vec<String> = schema
        .columns
        .iter()
        .map(|c| c.ty.history_expr(&format!("`{}`", c.name)))
        .collect();
    let statement = Statement {
        sql: format!(
            "SELECT {} FROM {{table:Identifier}} ORDER BY 1",
            exprs.join(", ")
        ),
        params: vec![("table".to_owned(), table.to_owned())],
    };
    history.fetch(&statement, &schema).await
}

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr io-tests`"]
async fn query_sends_params_settings_and_database() {
    let db = Database::new().await;
    let statement = Statement {
        sql: "SELECT {s:String} AS s, {n:UInt64} AS n, \
              getSetting('date_time_output_format') AS format, currentDatabase() AS db"
            .to_owned(),
        params: vec![
            ("s".to_owned(), "it's \"q\" ü".to_owned()),
            ("n".to_owned(), u64::MAX.to_string()),
        ],
    };
    let body = db.client.query(&statement, "JSONEachRow").await.unwrap();
    let row: Json = serde_json::from_str(body.trim()).unwrap();
    assert_eq!(
        row,
        json!({
            "s": "it's \"q\" ü",
            "n": u64::MAX.to_string(),
            "format": "unix_timestamp",
            "db": db.name,
        })
    );
    db.drop().await;
}

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr io-tests`"]
async fn server_error_keeps_status_and_message() {
    let db = Database::new().await;
    let err = db
        .client
        .query(&statement("SELECT nope"), "JSONEachRow")
        .await
        .unwrap_err();
    assert!(
        matches!(&err, ClickhouseError::ServerError { status, body }
            if *status >= 400 && body.contains("UNKNOWN_IDENTIFIER")),
        "{err:?}"
    );
    db.drop().await;
}

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr io-tests`"]
async fn error_after_first_rows_is_an_error() {
    let db = Database::new().await;
    let sql = "SELECT throwIf(number = 50000) FROM numbers(100000) SETTINGS max_block_size = 1000";
    let err = db
        .client
        .query(&statement(sql), "JSONEachRow")
        .await
        .unwrap_err();
    assert!(
        matches!(&err, ClickhouseError::ServerError { body, .. } if body.contains("FUNCTION_THROW_IF_VALUE_IS_NON_ZERO")),
        "{err:?}"
    );
    db.drop().await;
}

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr io-tests`"]
async fn wrong_user_is_rejected() {
    let client = ClickhouseClient::new(&config::ClickhouseConfig {
        user: "nope".to_owned(),
        ..clickhouse_config("default")
    });
    let err = client.execute(&statement("SELECT 1")).await.unwrap_err();
    assert!(
        matches!(&err, ClickhouseError::ServerError { body, .. } if body.contains("AUTHENTICATION_FAILED")),
        "{err:?}"
    );
}

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr io-tests`"]
async fn unreachable_server_is_a_request_error() {
    let client = ClickhouseClient::new(&config::ClickhouseConfig {
        url: "http://127.0.0.1:1".to_owned(),
        ..clickhouse_config("default")
    });
    let err = client.execute(&statement("SELECT 1")).await.unwrap_err();
    assert!(matches!(err, ClickhouseError::RequestError(_)), "{err:?}");
}

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr io-tests`"]
async fn describe_reads_columns_defaults_and_engine() {
    let db = Database::new().await;
    db.run(
        "CREATE TABLE `odd name-1` (
            id UInt64,
            name LowCardinality(Nullable(String)) DEFAULT 'x',
            upper String MATERIALIZED upper(ifNull(name, '')),
            alias String ALIAS 'a',
            eph UInt8 EPHEMERAL,
            spot Point
        ) ENGINE = MergeTree ORDER BY id",
    )
    .await;
    let description = history(&db).describe("odd name-1").await.unwrap();
    let column = |name: &str, ty: &str, default| DescribedColumn {
        name: name.to_owned(),
        ty: DataType::parse(ty),
        default,
    };
    assert_eq!(
        description.columns,
        vec![
            column("id", "UInt64", ColumnDefault::None),
            column("name", "Nullable(String)", ColumnDefault::Default),
            column("upper", "String", ColumnDefault::Materialized),
            column("alias", "String", ColumnDefault::Alias),
            column("eph", "UInt8", ColumnDefault::Ephemeral),
            column("spot", "Point", ColumnDefault::None),
        ]
    );
    assert_eq!(
        description.columns[5].ty,
        DataType::Other("Point".to_owned())
    );
    assert_eq!(description.engine, TableEngine::MergeTree);
    db.drop().await;
}

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr io-tests`"]
async fn describe_reads_replacing_merge_tree() {
    let db = Database::new().await;
    db.run(
        "CREATE TABLE versioned (a UInt64, b String, ver UInt32, del UInt8)
         ENGINE = ReplacingMergeTree(ver, del) ORDER BY (a, cityHash64(b, a))",
    )
    .await;
    db.run("CREATE TABLE plain (a UInt64) ENGINE = ReplacingMergeTree ORDER BY a")
        .await;
    db.run("CREATE TABLE memory (a UInt64) ENGINE = Memory")
        .await;
    let history = history(&db);
    let engine = |key: &[&str], version: Option<&str>, is_deleted: Option<&str>| {
        TableEngine::ReplacingMergeTree(ReplacingMergeTree {
            key: key.iter().map(|k| k.to_string()).collect(),
            version: version.map(str::to_owned),
            is_deleted: is_deleted.map(str::to_owned),
        })
    };
    assert_eq!(
        history.describe("versioned").await.unwrap().engine,
        engine(&["a", "cityHash64(b, a)"], Some("ver"), Some("del"))
    );
    assert_eq!(
        history.describe("plain").await.unwrap().engine,
        engine(&["a"], None, None)
    );
    assert_eq!(
        history.describe("memory").await.unwrap().engine,
        TableEngine::Other("Memory".to_owned())
    );
    db.drop().await;
}

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr io-tests`"]
async fn describe_only_sees_its_database() {
    let (db, other) = (Database::new().await, Database::new().await);
    other
        .run("CREATE TABLE t (a UInt64) ENGINE = MergeTree ORDER BY a")
        .await;
    let err = history(&db).describe("t").await.unwrap_err();
    assert!(
        matches!(&err, Error::Clickhouse(ClickhouseError::UnknownTable(t)) if t == "t"),
        "{err:?}"
    );
    db.drop().await;
    other.drop().await;
}

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr io-tests`"]
async fn fetch_decodes_every_type_exactly() {
    let db = Database::new().await;
    db.run(
        "CREATE TABLE all_types (
            id UInt64,
            s String,
            n Nullable(Int32),
            wide Int128,
            f Float64,
            b Bool,
            d Date32,
            t DateTime64(3, 'UTC'),
            dec Decimal(10, 2),
            u UUID,
            ip IPv6,
            arr Array(Date32),
            m Map(String, Date32),
            tup Tuple(a Date32, b String),
            e Enum8('a' = 1, 'b' = 2),
            fs FixedString(3)
        ) ENGINE = MergeTree ORDER BY id",
    )
    .await;
    db.run("INSERT INTO all_types (id) VALUES (0)").await;
    db.run(
        "INSERT INTO all_types VALUES (
            18446744073709551615, 'héllo \"q\"\\n', NULL, -170141183460469231731687303715884105728,
            1.5e300, true, '1900-01-01', '1969-12-31 23:59:59.500', -12345.67,
            '00000000-0000-0000-0000-000000000001', '::ffff:1.2.3.4', ['2299-12-31'],
            {'k': '1900-01-01'}, ('1970-01-02', 'x'), 'b', 'ab'
        )",
    )
    .await;
    let rows = fetch_all(&db, "all_types").await.unwrap();
    assert_eq!(
        rows,
        vec![
            vec![
                Value::Int(0),
                Value::String(String::new()),
                Value::Null,
                Value::BigInt(I256::ZERO),
                Value::Float(0.0),
                Value::Bool(false),
                Value::Date(-25567),
                Value::DateTime(0),
                Value::Decimal(I256::ZERO),
                Value::Uuid(0),
                Value::Ipv6(0.into()),
                Value::Array(Vec::new()),
                Value::Map(Vec::new()),
                Value::Tuple(vec![Value::Date(-25567), Value::String(String::new())]),
                Value::Enum(1),
                Value::String("\0\0\0".to_owned()),
            ],
            vec![
                Value::Int(u64::MAX.into()),
                Value::String("héllo \"q\"\n".to_owned()),
                Value::Null,
                Value::BigInt(I256::from(i128::MIN)),
                Value::Float(1.5e300),
                Value::Bool(true),
                Value::Date(-25567),
                Value::DateTime(-500),
                Value::Decimal(I256::from(-1234567)),
                Value::Uuid(1),
                Value::Ipv6(Ipv4Addr::new(1, 2, 3, 4).to_ipv6_mapped()),
                Value::Array(vec![Value::Date(120529)]),
                Value::Map(vec![(Value::String("k".to_owned()), Value::Date(-25567))]),
                Value::Tuple(vec![Value::Date(1), Value::String("x".to_owned())]),
                Value::Enum(2),
                Value::String("ab\0".to_owned()),
            ],
        ]
    );
    db.drop().await;
}

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr io-tests`"]
async fn fetch_of_empty_table_has_no_rows() {
    let db = Database::new().await;
    db.run("CREATE TABLE empty (a UInt64) ENGINE = MergeTree ORDER BY a")
        .await;
    assert_eq!(fetch_all(&db, "empty").await.unwrap(), Vec::<Row>::new());
    db.drop().await;
}

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr io-tests`"]
async fn fetch_of_bad_statement_is_a_server_error() {
    let db = Database::new().await;
    let schema = Schema {
        columns: vec![Column::new("a", DataType::parse("UInt64"))],
    };
    let err = history(&db)
        .fetch(&statement("SELECT a FROM missing"), &schema)
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::Clickhouse(ClickhouseError::ServerError { body, .. }) if body.contains("UNKNOWN_TABLE")),
        "{err:?}"
    );
    db.drop().await;
}

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr io-tests`"]
async fn clone_keeps_the_parts_it_attached() {
    let db = Database::new().await;
    db.run("CREATE TABLE src (id UInt64, p UInt8) ENGINE = MergeTree PARTITION BY p ORDER BY id")
        .await;
    db.run("INSERT INTO src VALUES (1, 1), (2, 1), (3, 2), (4, 2)")
        .await;
    // The source table can't be an `Identifier` parameter in either statement.
    db.run("CREATE TABLE `clone` AS `src`").await;
    db.run("ALTER TABLE `clone` ATTACH PARTITION ALL FROM `src`")
        .await;
    db.run("INSERT INTO src VALUES (5, 1), (6, 3)").await;

    let ids = |rows: Vec<Row>| -> Vec<Value> { rows.into_iter().map(|r| r[0].clone()).collect() };
    assert_eq!(
        ids(fetch_all(&db, "clone").await.unwrap()),
        (1..=4).map(Value::Int).collect::<Vec<_>>()
    );
    assert_eq!(
        ids(fetch_all(&db, "src").await.unwrap()),
        (1..=6).map(Value::Int).collect::<Vec<_>>()
    );
    let history = history(&db);
    let (clone, src) = (
        history.describe("clone").await.unwrap(),
        history.describe("src").await.unwrap(),
    );
    assert_eq!(clone, src);
    db.drop().await;
}
