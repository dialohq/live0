use config::TableConfig;
use io::{PartitionOffsets, TableDescription, TableEngine};

use crate::{
    Column, ColumnSecurity, Error, RowSecurity, SchemaHash, Table, TableKind, TableSecurity,
};

/// Checks the described table and its topic against the config and resolves names to columns.
pub(crate) fn build(
    config: &TableConfig,
    description: TableDescription,
    partitions: Vec<PartitionOffsets>,
) -> Result<Table, Error> {
    let invalid = |message: String| Error::InvalidTable {
        table: config.name.clone(),
        message,
    };
    let topic = config.topic.clone().unwrap_or_else(|| config.name.clone());
    let mut partitions: Vec<_> = partitions.iter().map(|p| p.partition).collect();
    partitions.sort_unstable();
    if partitions.is_empty() {
        return Err(invalid(format!("topic `{topic}` has no partitions")));
    }
    let columns: Vec<Column> = description
        .columns
        .into_iter()
        .map(|c| Column {
            name: c.name,
            ty: c.ty,
            default: c.default,
        })
        .collect();
    let id = |name: &str, role: &str| {
        columns
            .iter()
            .position(|c| c.name == name)
            .ok_or_else(|| invalid(format!("{role} `{name}` isn't a column")))
    };

    let kind = match description.engine {
        TableEngine::MergeTree => TableKind::Plain,
        TableEngine::ReplacingMergeTree(engine) => {
            if engine.key.is_empty() {
                return Err(invalid(
                    "ReplacingMergeTree needs an ORDER BY key".to_owned(),
                ));
            }
            TableKind::Replacing {
                key: engine
                    .key
                    .iter()
                    .map(|k| id(k, "key"))
                    .collect::<Result<_, _>>()?,
                version: engine
                    .version
                    .as_deref()
                    .map(|v| id(v, "version"))
                    .transpose()?,
                is_deleted: engine
                    .is_deleted
                    .as_deref()
                    .map(|d| id(d, "is_deleted"))
                    .transpose()?,
            }
        }
        TableEngine::Other(engine) => {
            return Err(invalid(format!("engine `{engine}` isn't supported")));
        }
    };

    let security = TableSecurity {
        grant: config.grant.clone(),
        columns: config
            .columns
            .iter()
            .map(|rule| {
                Ok(ColumnSecurity {
                    column: id(&rule.column, "secured column")?,
                    grant: rule.grant.clone(),
                })
            })
            .collect::<Result<_, Error>>()?,
        rows: config
            .rows
            .iter()
            .map(|rule| {
                Ok(RowSecurity {
                    column: id(&rule.column, "row rule column")?,
                    grant: rule.grant.clone(),
                })
            })
            .collect::<Result<_, Error>>()?,
    };

    let schema_hash = schema_hash(&(
        &config.name,
        &topic,
        &partitions,
        &columns,
        &kind,
        &security,
    ));
    Ok(Table {
        name: config.name.clone(),
        topic,
        partitions,
        columns,
        kind,
        security,
        schema_hash,
    })
}

/// FNV-1a of the `Debug` text, so it's the same in every process that reads a persisted cursor.
fn schema_hash(parts: &impl std::fmt::Debug) -> SchemaHash {
    let hash = format!("{parts:?}")
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        });
    SchemaHash(hash)
}

#[cfg(test)]
mod tests {
    use config::{ColumnRule, RowRule};
    use io::{ColumnDefault, DescribedColumn, ReplacingMergeTree};
    use types::DataType;

    use super::*;

    fn config(name: &str) -> TableConfig {
        TableConfig {
            name: name.to_owned(),
            topic: None,
            grant: None,
            columns: Vec::new(),
            rows: Vec::new(),
        }
    }

    fn description(engine: TableEngine) -> TableDescription {
        let column = |name: &str, ty: &str, default| DescribedColumn {
            name: name.to_owned(),
            ty: DataType::parse(ty),
            default,
        };
        TableDescription {
            columns: vec![
                column("id", "UInt64", ColumnDefault::None),
                column("name", "String", ColumnDefault::Default),
                column("ver", "UInt32", ColumnDefault::None),
                column("upper", "String", ColumnDefault::Materialized),
                column("spot", "Point", ColumnDefault::None),
            ],
            engine,
        }
    }

    fn partitions(ids: &[i32]) -> Vec<PartitionOffsets> {
        ids.iter()
            .map(|&partition| PartitionOffsets {
                partition,
                low: 0,
                high: 0,
            })
            .collect()
    }

    fn replacing(key: &[&str], version: Option<&str>, is_deleted: Option<&str>) -> TableEngine {
        TableEngine::ReplacingMergeTree(ReplacingMergeTree {
            key: key.iter().map(|k| k.to_string()).collect(),
            version: version.map(str::to_owned),
            is_deleted: is_deleted.map(str::to_owned),
        })
    }

    fn plain(config: &TableConfig) -> Result<Table, Error> {
        build(
            config,
            description(TableEngine::MergeTree),
            partitions(&[0]),
        )
    }

    fn invalid(result: Result<Table, Error>, expected: &str) {
        match result {
            Err(Error::InvalidTable { table, message }) => {
                assert_eq!(table, "events");
                assert!(message.contains(expected), "{message}");
            }
            other => panic!("expected InvalidTable, got {other:?}"),
        }
    }

    #[test]
    fn plain_table_keeps_every_column() {
        let table = plain(&config("events")).unwrap();
        assert_eq!(table.kind, TableKind::Plain);
        assert_eq!(
            table
                .columns
                .iter()
                .map(|c| (c.name.as_str(), c.default))
                .collect::<Vec<_>>(),
            [
                ("id", ColumnDefault::None),
                ("name", ColumnDefault::Default),
                ("ver", ColumnDefault::None),
                ("upper", ColumnDefault::Materialized),
                ("spot", ColumnDefault::None),
            ]
        );
        assert_eq!(table.columns[4].ty, DataType::Other("Point".to_owned()));
        assert_eq!(table.security, TableSecurity::default());
    }

    #[test]
    fn topic_defaults_to_name() {
        let table = build(
            &config("events"),
            description(TableEngine::MergeTree),
            partitions(&[2, 0, 1]),
        )
        .unwrap();
        assert_eq!(table.topic, "events");
        assert_eq!(table.partitions, [0, 1, 2]);
        let configured = TableConfig {
            topic: Some("raw".to_owned()),
            ..config("events")
        };
        assert_eq!(plain(&configured).unwrap().topic, "raw");
    }

    #[test]
    fn no_partitions_is_invalid() {
        invalid(
            build(
                &config("events"),
                description(TableEngine::MergeTree),
                Vec::new(),
            ),
            "topic `events` has no partitions",
        );
    }

    #[test]
    fn replacing_resolves_columns() {
        let build = |engine| {
            build(&config("events"), description(engine), partitions(&[0]))
                .unwrap()
                .kind
        };
        assert_eq!(
            build(replacing(&["name", "id"], Some("ver"), Some("id"))),
            TableKind::Replacing {
                key: vec![1, 0],
                version: Some(2),
                is_deleted: Some(0),
            }
        );
        assert_eq!(
            build(replacing(&["id"], None, None)),
            TableKind::Replacing {
                key: vec![0],
                version: None,
                is_deleted: None,
            }
        );
    }

    #[test]
    fn replacing_key_expression_is_invalid() {
        invalid(
            build(
                &config("events"),
                description(replacing(&["id", "cityHash64(name, id)"], None, None)),
                partitions(&[0]),
            ),
            "key `cityHash64(name, id)` isn't a column",
        );
        invalid(
            build(
                &config("events"),
                description(replacing(&["id"], Some("nope"), None)),
                partitions(&[0]),
            ),
            "version `nope`",
        );
    }

    #[test]
    fn replacing_without_key_is_invalid() {
        invalid(
            build(
                &config("events"),
                description(replacing(&[], None, None)),
                partitions(&[0]),
            ),
            "needs an ORDER BY key",
        );
    }

    #[test]
    fn other_engine_is_invalid() {
        invalid(
            build(
                &config("events"),
                description(TableEngine::Other("Memory".to_owned())),
                partitions(&[0]),
            ),
            "engine `Memory` isn't supported",
        );
    }

    #[test]
    fn security_resolves_columns() {
        let secured = TableConfig {
            grant: Some("read".to_owned()),
            columns: vec![ColumnRule {
                column: "name".to_owned(),
                grant: "pii".to_owned(),
            }],
            rows: vec![RowRule {
                column: "ver".to_owned(),
                grant: "versions".to_owned(),
            }],
            ..config("events")
        };
        assert_eq!(
            plain(&secured).unwrap().security,
            TableSecurity {
                grant: Some("read".to_owned()),
                columns: vec![ColumnSecurity {
                    column: 1,
                    grant: "pii".to_owned(),
                }],
                rows: vec![RowSecurity {
                    column: 2,
                    grant: "versions".to_owned(),
                }],
            }
        );
    }

    #[test]
    fn security_unknown_column_is_invalid() {
        let secured = TableConfig {
            rows: vec![RowRule {
                column: "tenant".to_owned(),
                grant: "tenants".to_owned(),
            }],
            ..config("events")
        };
        invalid(plain(&secured), "`tenant` isn't a column");
    }

    #[test]
    fn schema_hash_tracks_what_plans_depend_on() {
        let hash = |config: &TableConfig, description: TableDescription, ids: &[i32]| {
            build(config, description, partitions(ids))
                .unwrap()
                .schema_hash
        };
        let base = hash(&config("events"), description(TableEngine::MergeTree), &[0]);
        assert_eq!(
            base,
            hash(&config("events"), description(TableEngine::MergeTree), &[0])
        );

        let mut retyped = description(TableEngine::MergeTree);
        retyped.columns[0].ty = DataType::parse("Int64");
        let mut redefaulted = description(TableEngine::MergeTree);
        redefaulted.columns[0].default = ColumnDefault::Default;
        let secured = TableConfig {
            grant: Some("read".to_owned()),
            ..config("events")
        };
        for changed in [
            hash(&config("events"), retyped, &[0]),
            hash(&config("events"), redefaulted, &[0]),
            hash(
                &config("events"),
                description(TableEngine::MergeTree),
                &[0, 1],
            ),
            hash(
                &config("events"),
                description(replacing(&["id"], None, None)),
                &[0],
            ),
            hash(&secured, description(TableEngine::MergeTree), &[0]),
        ] {
            assert_ne!(base, changed);
        }
    }
}
