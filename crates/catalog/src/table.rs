use io::ColumnDefault;
use types::{DataType, Partition};

/// A column's position in its table's `columns`.
pub type ColumnId = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SchemaHash(pub u64);

#[derive(Debug, Clone)]
pub struct Table {
    pub name: String,
    pub topic: String,
    pub partitions: Vec<Partition>,
    pub columns: Vec<Column>,
    pub kind: TableKind,
    pub security: TableSecurity,
    /// Changes whenever anything a plan depends on changes, so it goes into plan fingerprints.
    pub schema_hash: SchemaHash,
}

impl Table {
    pub fn column(&self, name: &str) -> Option<(ColumnId, &Column)> {
        self.columns
            .iter()
            .enumerate()
            .find(|(_, c)| c.name == name)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub name: String,
    pub ty: DataType,
    pub default: ColumnDefault,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableKind {
    Plain,
    Replacing {
        key: Vec<ColumnId>,
        version: Option<ColumnId>,
        is_deleted: Option<ColumnId>,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TableSecurity {
    /// A check grant needed to query the table.
    pub grant: Option<String>,
    pub columns: Vec<ColumnSecurity>,
    pub rows: Vec<RowSecurity>,
}

/// The column is hidden without the check grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnSecurity {
    pub column: ColumnId,
    pub grant: String,
}

/// Only rows whose column holds one of the subject's values for the value grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowSecurity {
    pub column: ColumnId,
    pub grant: String,
}
