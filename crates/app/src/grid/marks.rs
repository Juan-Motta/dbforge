//! What an editing owner (a table's tab) draws on top of a grid's values:
//! new rows and rows marked for deletion, edited cells and cells left to
//! their DEFAULT, links on foreign key cells, and the header's marks
//! (primary key, computed, sort order). A read-only result has none.

use std::collections::{HashMap, HashSet};

use super::selection::Cell;

/// What a row is among pending changes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RowState {
    #[default]
    Loaded,
    /// A new row, not in the table yet.
    Inserted,
    /// A loaded row marked for deletion.
    Deleted,
}

/// What a cell is among pending changes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CellMark {
    /// Holds a value not applied yet.
    pub edited: bool,
    /// Left to its column's DEFAULT: drawn `DEFAULT`.
    pub unset: bool,
}

/// Which way a column's rows are ordered, as its header shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortMark {
    Asc,
    Desc,
}

/// What a column's header shows besides its name and type.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ColumnMark {
    /// Part of the primary key: a key before the name.
    pub key: bool,
    /// Computed by the database: a badge after the name.
    pub generated: bool,
    /// The arrow at the header's end, kept while unsorted so the column keeps its width.
    pub sort: Option<SortMark>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GridMarks {
    /// Each row's label in the gutter (its number in the table, `+`, `−`).
    pub row_labels: Vec<String>,
    /// The characters the gutter has room for, so it keeps its width whatever is pending.
    pub gutter_chars: usize,
    pub rows: Vec<RowState>,
    pub cells: HashMap<Cell, CellMark>,
    /// Cells that open the row their foreign key points to.
    pub links: HashSet<Cell>,
    pub columns: Vec<ColumnMark>,
    /// A header click sorts by its column.
    pub sortable: bool,
    /// What the header's computed badge says.
    pub computed_badge: &'static str,
}

impl GridMarks {
    pub fn row(&self, row: usize) -> RowState {
        self.rows.get(row).copied().unwrap_or_default()
    }

    pub fn cell(&self, cell: Cell) -> CellMark {
        self.cells.get(&cell).copied().unwrap_or_default()
    }

    pub fn column(&self, column: usize) -> ColumnMark {
        self.columns.get(column).cloned().unwrap_or_default()
    }
}
