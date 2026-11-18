//! Which cells of a grid are selected, as the web grid picked them: a click
//! picks one cell, ⌘/Ctrl adds or removes one, Shift (or a drag) a block
//! from the anchor; the arrows move a cell at a time and Shift+arrows grow
//! the block. The selection copies as tab-separated text for a spreadsheet.

use std::collections::BTreeSet;

/// A cell by its row and column, as the grid shows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Cell {
    pub row: usize,
    pub column: usize,
}

impl Cell {
    pub fn new(row: usize, column: usize) -> Self {
        Self { row, column }
    }
}

/// How a click picks cells: alone, added or removed (⌘/Ctrl), or a block
/// from the anchor (Shift, or dragging).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Single,
    Toggle,
    Range,
}

impl Mode {
    /// The mode a click's modifiers ask for; `secondary` is ⌘ on macOS, Ctrl elsewhere.
    pub fn of(shift: bool, secondary: bool) -> Self {
        if shift {
            Self::Range
        } else if secondary {
            Self::Toggle
        } else {
            Self::Single
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    /// The selected cells, ordered by row then column.
    cells: BTreeSet<Cell>,
    /// Where a block starts.
    anchor: Option<Cell>,
    /// The cell last picked: where the arrows move from.
    cursor: Option<Cell>,
}

impl Selection {
    pub fn cells(&self) -> &BTreeSet<Cell> {
        &self.cells
    }

    pub fn contains(&self, cell: Cell) -> bool {
        self.cells.contains(&cell)
    }

    pub fn cursor(&self) -> Option<Cell> {
        self.cursor
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Picks `target` as `mode` says; `whole_row` picks each of `columns`
    /// columns of the rows instead (a click on a row's number).
    pub fn select(&mut self, target: Cell, mode: Mode, columns: usize, whole_row: bool) {
        let span_columns = |from: usize, to: usize| if whole_row { 0..columns } else { from.min(to)..from.max(to) + 1 };
        match mode {
            Mode::Single => {
                self.cells =
                    span_columns(target.column, target.column).map(|column| Cell::new(target.row, column)).collect();
                self.anchor = Some(target);
            }
            Mode::Toggle => {
                let cells: Vec<Cell> =
                    span_columns(target.column, target.column).map(|column| Cell::new(target.row, column)).collect();
                let all = cells.iter().all(|cell| self.cells.contains(cell));
                for cell in cells {
                    if all {
                        self.cells.remove(&cell);
                    } else {
                        self.cells.insert(cell);
                    }
                }
                self.anchor = Some(target);
            }
            Mode::Range => {
                let anchor = self.anchor.unwrap_or(target);
                let rows = anchor.row.min(target.row)..anchor.row.max(target.row) + 1;
                self.cells = rows
                    .flat_map(|row| {
                        span_columns(anchor.column, target.column).map(move |column| Cell::new(row, column))
                    })
                    .collect();
                self.anchor = Some(anchor);
            }
        }
        self.cursor = Some(target);
    }

    /// Every cell of a `rows` × `columns` grid.
    pub fn select_all(&mut self, rows: usize, columns: usize) {
        self.cells = (0..rows).flat_map(|row| (0..columns).map(move |column| Cell::new(row, column))).collect();
        self.anchor = (rows > 0 && columns > 0).then_some(Cell::new(0, 0));
        self.cursor = self.anchor;
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// The arrows: moves the cursor `rows` down and `columns` right, within
    /// a `height` × `width` grid; with `extend` the block from the anchor
    /// grows to it, otherwise it alone is selected. The cell moved to.
    pub fn step(&mut self, rows: isize, columns: isize, extend: bool, height: usize, width: usize) -> Option<Cell> {
        if height == 0 || width == 0 {
            return None;
        }
        let from = self.cursor.unwrap_or(Cell::new(0, 0));
        let moved = |at: usize, by: isize, size: usize| (at as isize + by).clamp(0, size as isize - 1) as usize;
        let target = if self.cursor.is_none() {
            from
        } else {
            Cell::new(moved(from.row, rows, height), moved(from.column, columns, width))
        };
        self.select(target, if extend { Mode::Range } else { Mode::Single }, width, false);
        Some(target)
    }

    /// The rows with a selected cell, in order.
    pub fn rows(&self) -> Vec<usize> {
        let mut rows: Vec<usize> = self.cells.iter().map(|cell| cell.row).collect();
        rows.dedup();
        rows
    }

    /// The selection as tab-separated text: one line per row with a
    /// selected cell, one field per column with one, empty where a cell of
    /// that block isn't selected. NULL is written `NULL`; a value with a
    /// tab, a line break or a quote is quoted.
    pub fn as_tsv<'a>(&self, value: impl Fn(Cell) -> Option<&'a str>) -> String {
        let columns: BTreeSet<usize> = self.cells.iter().map(|cell| cell.column).collect();
        let field = |cell: Cell| {
            let text = value(cell).unwrap_or("NULL");
            if text.contains(['\t', '\n', '\r', '"']) {
                format!("\"{}\"", text.replace('"', "\"\""))
            } else {
                text.to_owned()
            }
        };
        self.rows()
            .into_iter()
            .map(|row| {
                let fields: Vec<String> = columns
                    .iter()
                    .map(|&column| {
                        let cell = Cell::new(row, column);
                        if self.cells.contains(&cell) { field(cell) } else { String::new() }
                    })
                    .collect();
                fields.join("\t")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(selection: &Selection) -> Vec<(usize, usize)> {
        selection.cells().iter().map(|cell| (cell.row, cell.column)).collect()
    }

    #[test]
    fn picks_one_cell_adds_and_removes_cells_and_selects_whole_rows() {
        let mut selection = Selection::default();
        selection.select(Cell::new(1, 1), Mode::Single, 3, false);
        assert_eq!(keys(&selection), [(1, 1)]);
        selection.select(Cell::new(3, 2), Mode::Toggle, 3, false);
        assert_eq!(keys(&selection), [(1, 1), (3, 2)]);
        selection.select(Cell::new(1, 1), Mode::Toggle, 3, false);
        assert_eq!(keys(&selection), [(3, 2)]);
        selection.select(Cell::new(2, 0), Mode::Single, 3, true);
        assert_eq!(keys(&selection), [(2, 0), (2, 1), (2, 2)]);
    }

    #[test]
    fn selects_a_block_from_the_anchor_either_way() {
        let mut selection = Selection::default();
        selection.select(Cell::new(2, 2), Mode::Single, 3, false);
        selection.select(Cell::new(0, 0), Mode::Range, 3, false);
        assert_eq!(keys(&selection).len(), 9);
        assert_eq!(selection.rows(), [0, 1, 2]);
        let mut rows = Selection::default();
        rows.select(Cell::new(1, 0), Mode::Single, 2, true);
        rows.select(Cell::new(3, 0), Mode::Range, 2, true);
        assert_eq!(keys(&rows), [(1, 0), (1, 1), (2, 0), (2, 1), (3, 0), (3, 1)]);
    }

    #[test]
    fn reads_the_mode_from_the_modifier_keys() {
        assert_eq!(Mode::of(true, true), Mode::Range);
        assert_eq!(Mode::of(false, true), Mode::Toggle);
        assert_eq!(Mode::of(false, false), Mode::Single);
    }

    #[test]
    fn arrows_move_the_cell_and_shift_grows_the_block_within_the_grid() {
        let mut selection = Selection::default();
        assert_eq!(selection.step(1, 0, false, 5, 3), Some(Cell::new(0, 0)), "the first arrow picks the first cell");
        selection.step(1, 1, false, 5, 3);
        assert_eq!(keys(&selection), [(1, 1)]);
        selection.step(1, 1, true, 5, 3);
        assert_eq!(keys(&selection), [(1, 1), (1, 2), (2, 1), (2, 2)]);
        selection.step(9, 9, false, 5, 3);
        assert_eq!(keys(&selection), [(4, 2)], "clamped to the last cell");
        assert_eq!(Selection::default().step(1, 0, false, 0, 3), None);
    }

    #[test]
    fn copies_the_selected_block_for_a_spreadsheet() {
        let values = |cell: Cell| match (cell.row, cell.column) {
            (1, 0) => Some("1"),
            (1, 1) => Some("a\tb"),
            (3, 2) => Some("say \"hi\""),
            _ => None,
        };
        let mut selection = Selection::default();
        for cell in [Cell::new(1, 0), Cell::new(1, 1), Cell::new(3, 1), Cell::new(3, 2)] {
            selection.select(cell, Mode::Toggle, 3, false);
        }
        assert_eq!(selection.as_tsv(values), "1\t\"a\tb\"\t\n\tNULL\t\"say \"\"hi\"\"\"");
        selection.select_all(2, 2);
        assert_eq!(selection.cells().len(), 4);
    }
}
