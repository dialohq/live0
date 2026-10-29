use async_trait::async_trait;
use types::DecodeSpec;

mod clickhouse;

pub const HISTORY_FORMAT: &str = "JSONCompactEachRowWithNamesAndTypes";
pub const HISTORY_SETTINGS: &[(&str, &str)] = &[
    ("output_format_json_quote_64bit_integers", "1"),
    ("output_format_json_quote_denormals", "1"),
];

pub struct RMT {
    sort_key: String,
    ver_key: String,
    is_deleted_key: Option<String>,
}

pub enum TableEngine {
    MergeTree,
    ReplacingMergeTree(RMT),
    Other(String),
}

pub struct DescribedTable {
    pub columns: DecodeSpec,
    pub engine: TableEngine,
}

#[async_trait]
pub trait HistorySource: Sync + Send {
    type Error;

    async fn fetch(&self, spec: DecodeSpec, sql: &str) -> Result<Vec<types::Row>, Self::Error>;

    async fn describe(&self, table: &str) -> Result<DescribedTable, Self::Error>;
}
