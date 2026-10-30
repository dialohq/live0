use catalog::Catalog;
use config::{ColumnRule, RowRule, TableConfig, TablesConfig};
use io::{ColumnDefault, DescribedColumn, FakeCluster, TableDescription, TableEngine};
use types::DataType;

use crate::{Access, Grant};

/// `dates` reads the configured topic `raw.dates`; every other table reads its own name.
fn topic(table: &str) -> String {
    match table {
        "dates" => "raw.dates".to_owned(),
        table => table.to_owned(),
    }
}

fn description(columns: &[(&str, &str, ColumnDefault)]) -> TableDescription {
    TableDescription {
        columns: columns
            .iter()
            .map(|(name, ty, default)| DescribedColumn {
                name: (*name).to_owned(),
                ty: DataType::parse(ty),
                default: *default,
            })
            .collect(),
        engine: TableEngine::MergeTree,
    }
}

fn table_config(name: &str) -> TableConfig {
    TableConfig {
        name: name.to_owned(),
        topic: None,
        grant: None,
        columns: Vec::new(),
        rows: Vec::new(),
    }
}

pub(crate) async fn catalog() -> Catalog {
    use ColumnDefault::*;
    let cluster = FakeCluster::new();
    let tables = [
        (
            "events",
            description(&[
                ("id", "UInt64", None),
                ("name", "String", Default),
                ("email", "String", None),
                ("upper", "String", Materialized),
                ("alias_name", "String", Alias),
                ("eph", "UInt8", Ephemeral),
            ]),
        ),
        (
            "geo",
            description(&[("id", "UInt64", None), ("spot", "Point", None)]),
        ),
        (
            "tenants",
            description(&[("id", "UInt64", None), ("tenant", "String", None)]),
        ),
        (
            "dates",
            description(&[("id", "UInt64", None), ("d", "Date32", None)]),
        ),
    ];
    for (name, description) in tables {
        let topic = topic(name);
        cluster.create_topic(&topic, 1).unwrap();
        cluster.create_table(name, &topic, description).unwrap();
    }
    let config = TablesConfig {
        expose_all: false,
        tables: vec![
            TableConfig {
                grant: Some("events:read".to_owned()),
                columns: vec![ColumnRule {
                    column: "email".to_owned(),
                    grant: "pii".to_owned(),
                }],
                ..table_config("events")
            },
            table_config("geo"),
            TableConfig {
                rows: vec![RowRule {
                    column: "tenant".to_owned(),
                    grant: "tenant".to_owned(),
                }],
                ..table_config("tenants")
            },
            TableConfig {
                topic: Some(topic("dates")),
                ..table_config("dates")
            },
        ],
    };
    Catalog::load(&config, &cluster.history(), &cluster.stream())
        .await
        .unwrap()
}

pub(crate) fn grants(checks: &[&str]) -> Access {
    Access::Grants(
        checks
            .iter()
            .map(|g| (g.to_string(), Grant::Check(true)))
            .collect(),
    )
}
